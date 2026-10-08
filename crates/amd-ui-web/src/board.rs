//! The board: an Epics column, then the four ticket columns, with drag and
//! drop between neighbouring columns and between the backlog and its epics.

use amd_ui_model::epic::{self, GroupKind, GroupState, Place, can_move};
use amd_ui_model::wip::WipLimits;
use amd_ui_model::{Column, TicketSummary};
use leptos::ev::DragEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::app::{Ctx, Overlay};

#[component]
pub fn BoardView() -> impl IntoView {
    view! {
        <div class="flex h-full gap-3 overflow-x-auto p-3">
            <EpicsColumn />
            {move || {
                Ctx::get()
                    .open_epic
                    .get()
                    .map(|(repo, name)| view! { <OpenEpicColumn repo name /> })
            }}
            {Column::ALL.map(|column| view! { <ColumnView column /> }).collect_view()}
        </div>
    }
}

/// What a backlog container's own state means for the sprint rules.
fn group_state(ctx: Ctx, place: &Place) -> Option<GroupState> {
    let name = place.epic.as_ref()?;
    ctx.board.with(|b| {
        b.as_ref()?
            .epics
            .iter()
            .find(|e| e.repo == place.repo && e.name == *name)
            .map(|e| GroupState {
                kind: e.kind,
                started: e.started,
            })
    })
}

/// The ticket being dragged, looked up by where it is.
fn ticket_at(ctx: Ctx, file: &str, from: &Place) -> Option<TicketSummary> {
    ctx.board.with(|b| {
        let b = b.as_ref()?;
        let list = match &from.epic {
            Some(name) => b
                .epics
                .iter()
                .find(|e| e.repo == from.repo && e.name == *name)
                .map(|e| &e.tickets),
            None => Some(&b.columns[from.column.index()].tickets),
        };
        list.and_then(|l| l.iter().find(|t| t.file == file && t.repo == from.repo))
            .cloned()
    })
}

/// Whether the dragged ticket may go to `to`: the column and epic rules,
/// then the draft and sprint rules. An empty error means `to` is not a
/// target at all.
fn verdict(ctx: Ctx, file: &str, from: &Place, to: &Place) -> Result<(), String> {
    if !can_move(from, to) {
        return Err(String::new());
    }
    let ticket = ticket_at(ctx, file, from);
    let sized = ticket.as_ref().is_some_and(|t| epic::has_points(&t.points));
    let draft = ticket.as_ref().is_some_and(|t| epic::is_draft(&t.tags));
    let stem = file.trim_end_matches(".md");
    epic::draft_rule(from, to, group_state(ctx, to), draft, stem)?;
    epic::sprint_rule(
        from,
        to,
        group_state(ctx, from),
        group_state(ctx, to),
        sized,
        stem,
    )
}

/// Whether the ticket being dragged may be dropped at `to`.
fn accepts(ctx: Ctx, to: &Place) -> bool {
    ctx.dragging.with(|d| {
        d.as_ref()
            .is_some_and(|(file, from)| verdict(ctx, file, from, to).is_ok())
    })
}

/// Why `to` would refuse the ticket being dragged, when only the sprint
/// rules stand in the way (it is otherwise a valid target).
fn refusal(ctx: Ctx, to: &Place) -> Option<String> {
    ctx.dragging.with(|d| {
        let (file, from) = d.as_ref()?;
        verdict(ctx, file, from, to)
            .err()
            .filter(|why| !why.is_empty())
    })
}

/// Whether a drag is under way that `to` must refuse (and is not its source).
fn rejects(ctx: Ctx, to: &Place) -> bool {
    ctx.dragging.with(|d| {
        d.as_ref()
            .is_some_and(|(file, from)| from != to && verdict(ctx, file, from, to).is_err())
    })
}

/// `repo`'s WIP limit (on doing).
fn limits_of(ctx: Ctx, repo: &str) -> WipLimits {
    ctx.board.with(|b| {
        b.as_ref()
            .and_then(|b| b.repos.iter().find(|r| r.name == repo))
            .map(|r| r.wip)
            .unwrap_or_default()
    })
}

/// How many tickets `column` of `repo`'s board holds, whatever the filters
/// are showing.
fn held(ctx: Ctx, repo: &str, column: Column) -> usize {
    ctx.board.with(|b| {
        b.as_ref().map_or(0, |b| {
            b.columns[column.index()]
                .tickets
                .iter()
                .filter(|t| t.repo == repo)
                .count()
        })
    })
}

/// Why `to` is too full to take the ticket: its column is at its WIP limit.
/// The user can still move it there deliberately.
fn over_limit(ctx: Ctx, file: &str, from: &Place, to: &Place) -> Option<String> {
    let entering = to.epic.is_none() && (from.epic.is_some() || from.column != to.column);
    if !entering {
        return None;
    }
    let count = held(ctx, &to.repo, to.column);
    limits_of(ctx, &to.repo)
        .admit(to.column, count, file.trim_end_matches(".md"))
        .err()
}

/// Whether `to` is a valid target for the ticket being dragged but is full.
fn full_for_drag(ctx: Ctx, to: &Place) -> bool {
    ctx.dragging.with(|d| {
        d.as_ref()
            .is_some_and(|(file, from)| over_limit(ctx, file, from, to).is_some())
    })
}

/// Moves the dragged ticket to `to`, if that is allowed. Into a column at
/// its WIP limit, the user is asked first.
fn drop_on(ctx: Ctx, to: Place) {
    let Some((file, from)) = ctx.dragging.get_untracked() else {
        return;
    };
    ctx.dragging.set(None);
    if verdict(ctx, &file, &from, &to).is_err() {
        return;
    }
    if let Some(reason) = over_limit(ctx, &file, &from, &to) {
        ctx.overlay.set(Some(Overlay::OverLimit {
            file,
            from,
            to,
            reason,
        }));
        return;
    }
    move_ticket(ctx, file, from, to, false);
}

/// Moves a ticket, `force` overriding a full column's WIP limit.
pub fn move_ticket(ctx: Ctx, file: String, from: Place, to: Place, force: bool) {
    spawn_local(async move {
        match api::move_ticket(&file, &from, &to, force).await {
            Ok(report) => ctx.report(report),
            Err(e) => ctx.error.set(Some(e)),
        }
        ctx.reload();
    });
}

#[component]
fn ColumnView(column: Column) -> impl IntoView {
    let ctx = Ctx::get();
    // A column is shared by every repository: the place a dragged ticket
    // would land is this column on its own repository's board.
    let here = move || {
        ctx.dragging.with(|d| {
            let repo = d
                .as_ref()
                .map(|(_, from)| from.repo.clone())
                .unwrap_or_default();
            Place::column(column).in_repo(&repo)
        })
    };
    let tickets = move || {
        let list = ctx.board.with(|b| {
            b.as_ref()
                .map(|b| b.columns[column.index()].tickets.clone())
                .unwrap_or_default()
        });
        ctx.shown(list)
    };
    // What the column shows: only tickets passing the repository filter
    // and the search.
    let count = move || {
        ctx.board.with(|b| {
            b.as_ref().map_or(0, |b| {
                b.columns[column.index()]
                    .tickets
                    .iter()
                    .filter(|t| ctx.in_view(t))
                    .count()
            })
        })
    };
    let collapsed = move || ctx.collapsed.with(|c| c.contains(&column));
    let set_collapsed = move |hide: bool| {
        ctx.collapsed.update(|c| {
            if hide {
                c.insert(column);
            } else {
                c.remove(&column);
            }
        })
    };

    // Hide points towards the board edge the column folds into; show points
    // back out.
    let (hide_arrow, show_arrow) = if column == Column::Done {
        ("»", "«")
    } else {
        ("«", "»")
    };

    // While a card is being dragged, valid targets light up and every other
    // column fades.
    let on_dragover = move |ev: DragEvent| {
        if accepts(ctx, &here()) {
            // Cancelling dragover is what marks this column as a drop target.
            ev.prevent_default();
            if let Some(dt) = ev.data_transfer() {
                dt.set_drop_effect("move");
            }
        }
    };
    let on_drop = move |ev: DragEvent| {
        ev.prevent_default();
        // Read the target before drop_on clears the drag.
        let to = here();
        drop_on(ctx, to);
    };

    // The column's WIP limit, against what it holds: for the repository
    // being filtered to, or the board's own.
    let wip = move || {
        let repo = ctx.repo.get().or_else(|| {
            ctx.board.with(|b| {
                b.as_ref()
                    .and_then(|b| b.repos.first())
                    .map(|r| r.name.clone())
            })
        })?;
        let limit = limits_of(ctx, &repo).of(column)?;
        Some((held(ctx, &repo, column), limit as usize))
    };
    let full = move || full_for_drag(ctx, &here());

    let frame = move || {
        let to = here();
        let state = if accepts(ctx, &to) && full_for_drag(ctx, &to) {
            "ring-2 ring-amber-500 bg-amber-50 dark:bg-amber-950/60"
        } else if accepts(ctx, &to) {
            "ring-2 ring-accent-500 bg-accent-50 dark:bg-accent-950"
        } else if rejects(ctx, &to) {
            "opacity-40"
        } else {
            "bg-slate-200/70 dark:bg-slate-800/70"
        };
        format!("flex min-h-0 flex-col rounded-lg transition {state}")
    };

    view! {
        <Show
            when=collapsed
            fallback=move || {
                view! {
                    <section
                        aria-label=column.title()
                        class=move || format!("{} min-w-48 flex-1 basis-0", frame())
                        on:dragover=on_dragover
                        on:drop=on_drop
                    >
                        <header class="flex items-center gap-2 px-3 py-2">
                            <h2 class="text-sm font-semibold uppercase tracking-wide text-slate-600 dark:text-slate-300">
                                {column.title()}
                            </h2>
                            <span class="rounded-full bg-slate-300 px-2 text-xs dark:bg-slate-700">{count}</span>
                            {move || {
                                wip()
                                    .map(|(held, limit)| {
                                        let tone = if held > limit {
                                            "bg-red-200 text-red-900 dark:bg-red-900 dark:text-red-100"
                                        } else if held == limit {
                                            "bg-amber-200 text-amber-900 dark:bg-amber-900 dark:text-amber-100"
                                        } else {
                                            "bg-slate-300 dark:bg-slate-700"
                                        };
                                        view! {
                                            <span
                                                class=format!("rounded-full px-2 text-xs {tone}")
                                                title=format!(
                                                    "Work in progress: {held} of a limit of {limit}. Change the limit in Admin.",
                                                )
                                            >
                                                {format!("WIP {held}/{limit}")}
                                            </span>
                                        }
                                    })
                            }}
                            <span class="flex-1"></span>
                            <Show when=move || column.is_collapsible()>
                                <button
                                    class="rounded px-1.5 text-slate-500 hover:bg-slate-300 dark:hover:bg-slate-700"
                                    title=format!("Hide {}", column.title())
                                    aria-label=format!("Hide {}", column.title())
                                    on:click=move |_| set_collapsed(true)
                                >
                                    {hide_arrow}
                                </button>
                            </Show>
                        </header>
                        <Show when=full>
                            <p class="mx-2 mb-2 rounded bg-amber-100 px-2 py-1 text-xs text-amber-900 dark:bg-amber-900 dark:text-amber-100">
                                {format!(
                                    "{} is at its WIP limit. Drop here only to move it anyway: you will be asked to confirm.",
                                    column.title(),
                                )}
                            </p>
                        </Show>
                        <ul class="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-2 pb-2">
                            <For each=tickets key=|t| t.clone() children=|ticket| view! { <Card ticket /> } />
                        </ul>
                    </section>
                }
            }
        >
            <section
                aria-label=format!("{} (hidden)", column.title())
                class=move || format!("{} w-10 shrink-0 items-center", frame())
                on:dragover=on_dragover
                on:drop=on_drop
            >
                <button
                    class="flex h-full w-full flex-col items-center gap-2 rounded-lg py-3 hover:bg-slate-300 dark:hover:bg-slate-700"
                    title=format!("Show {}", column.title())
                    aria-label=format!("Show {}", column.title())
                    on:click=move |_| set_collapsed(false)
                >
                    <span class="text-slate-500">{show_arrow}</span>
                    <span class="rounded-full bg-slate-300 px-1.5 text-xs dark:bg-slate-700">{count}</span>
                    <span class="text-sm font-semibold uppercase tracking-wide text-slate-600 [writing-mode:vertical-rl] dark:text-slate-300">
                        {column.title()}
                    </span>
                </button>
            </section>
        </Show>
    }
}

/// The Backlogs column, left of the backlog: every sprint, then every epic,
/// each a box, with buttons to create them. It collapses on its own,
/// independently of the backlog beside it.
#[component]
fn EpicsColumn() -> impl IntoView {
    let ctx = Ctx::get();
    let groups = move || epics_in_view(ctx);
    let count = move || groups().len();
    let collapsed = move || ctx.epics_collapsed.get();
    let frame = "flex min-h-0 flex-col rounded-lg bg-slate-200/70 dark:bg-slate-800/70";
    let small_button = "whitespace-nowrap rounded px-1.5 text-xs text-slate-600 hover:bg-slate-300 dark:text-slate-300 dark:hover:bg-slate-700";

    view! {
        <Show
            when=collapsed
            fallback=move || {
                view! {
                    <section aria-label="Backlogs" class=format!("{frame} min-w-48 flex-1 basis-0")>
                        // Wraps its buttons under the title when the column is narrow.
                        <header class="flex flex-wrap items-center gap-1 px-3 py-2">
                            <h2 class="text-sm font-semibold uppercase tracking-wide text-slate-600 dark:text-slate-300">
                                "Backlogs"
                            </h2>
                            <span class="ml-1 rounded-full bg-slate-300 px-2 text-xs dark:bg-slate-700">{count}</span>
                            <span class="flex-1"></span>
                            <button
                                class=small_button
                                title="Create a sprint: a time-boxed backlog of sized tickets"
                                on:click=move |_| ctx.overlay.set(Some(Overlay::NewSprint))
                            >
                                "+ Sprint"
                            </button>
                            <button
                                class=small_button
                                title="Create an epic to group backlog tickets"
                                on:click=move |_| ctx.overlay.set(Some(Overlay::NewEpic))
                            >
                                "+ Epic"
                            </button>
                            <button
                                class="rounded px-1.5 text-slate-500 hover:bg-slate-300 dark:hover:bg-slate-700"
                                title="Hide Backlogs"
                                aria-label="Hide Backlogs"
                                on:click=move |_| ctx.epics_collapsed.set(true)
                            >
                                "«"
                            </button>
                        </header>
                        <div class="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-2 pb-2">
                            <For
                                each=groups
                                key=|group| group.clone()
                                children=|(repo, name, colour)| view! { <EpicBox repo name colour /> }
                            />
                            <Show when=move || count() == 0>
                                <p class="px-2 py-4 text-center text-xs text-slate-500">
                                    "No sprints or epics yet. Use + Sprint or + Epic to create one, then drag backlog tickets into it."
                                </p>
                            </Show>
                        </div>
                    </section>
                }
            }
        >
            <section aria-label="Backlogs (hidden)" class=format!("{frame} w-10 shrink-0 items-center")>
                <button
                    class="flex h-full w-full flex-col items-center gap-2 rounded-lg py-3 hover:bg-slate-300 dark:hover:bg-slate-700"
                    title="Show Backlogs"
                    aria-label="Show Backlogs"
                    on:click=move |_| ctx.epics_collapsed.set(false)
                >
                    <span class="text-slate-500">"»"</span>
                    <span class="rounded-full bg-slate-300 px-1.5 text-xs dark:bg-slate-700">{count}</span>
                    <span class="text-sm font-semibold uppercase tracking-wide text-slate-600 [writing-mode:vertical-rl] dark:text-slate-300">
                        "Backlogs"
                    </span>
                </button>
            </section>
        </Show>
    }
}

/// The sprint or epic called `name` in `repo`, as the board has it now.
fn group_memo(ctx: Ctx, repo: String, name: String) -> Memo<Option<amd_ui_model::Epic>> {
    Memo::new(move |_| {
        ctx.board.with(|b| {
            b.as_ref()
                .and_then(|b| b.epics.iter().find(|e| e.name == name && e.repo == repo))
                .cloned()
        })
    })
}

/// A sprint's total points. `+ 0.0` turns the empty sum's -0 into 0.
fn total_points(group: &amd_ui_model::Epic) -> f64 {
    group
        .tickets
        .iter()
        .filter_map(|t| t.points.trim().parse::<f64>().ok())
        .sum::<f64>()
        + 0.0
}

/// Lets a box or column take a dragged ticket when the rules allow it.
fn drop_target(
    ctx: Ctx,
    here: StoredValue<Place>,
) -> (impl Fn(DragEvent) + Copy, impl Fn(DragEvent) + Copy) {
    let on_dragover = move |ev: DragEvent| {
        ev.stop_propagation();
        if here.with_value(|to| accepts(ctx, to)) {
            ev.prevent_default();
            if let Some(dt) = ev.data_transfer() {
                dt.set_drop_effect("move");
            }
        }
    };
    let on_drop = move |ev: DragEvent| {
        ev.prevent_default();
        ev.stop_propagation();
        drop_on(ctx, here.get_value());
    };
    (on_dragover, on_drop)
}

/// A sprint or an epic in the Backlogs column (ticket 109): its name and
/// how much is in it — an epic's ticket count, a sprint's total points —
/// and nothing else. A click opens its tickets in a column of their own
/// ([`OpenEpicColumn`]); a backlog ticket can still be dropped straight
/// onto it, and while one it cannot take is dragged over it, it says why.
#[component]
fn EpicBox(repo: String, name: String, colour: usize) -> impl IntoView {
    let (tone, badge) = EPIC_COLOURS[colour % EPIC_COLOURS.len()];
    let ctx = Ctx::get();
    let here = StoredValue::new(Place::epic(&name).in_repo(&repo));
    let ids = StoredValue::new((repo.clone(), name.clone()));
    let group = group_memo(ctx, repo, name.clone());
    let is_sprint = move || group.with(|g| g.as_ref().is_some_and(|g| g.kind == GroupKind::Sprint));
    let shown = Memo::new(move |_| {
        let list = group.with(|g| g.as_ref().map(|g| g.tickets.clone()).unwrap_or_default());
        ctx.shown(list).len()
    });
    let amount = move || {
        group.with(|g| match g {
            Some(g) if g.kind == GroupKind::Sprint => format!("{} pts", total_points(g)),
            _ => shown.get().to_string(),
        })
    };
    let is_open = move || ctx.open_epic.with(|o| o.as_ref() == Some(&ids.get_value()));
    let refusal = move || here.with_value(|to| refusal(ctx, to));
    let (on_dragover, on_drop) = drop_target(ctx, here);
    // Sprints have a solid border, epics a dashed one; an outline on top
    // shows the box will take the drop, or that it's the one open.
    let frame = move || {
        let style = if is_sprint() {
            "border-solid"
        } else {
            "border-dashed"
        };
        let ring = if here.with_value(|to| accepts(ctx, to)) || is_open() {
            "ring-2 ring-accent-500 ring-offset-1 ring-offset-slate-200 dark:ring-offset-slate-800"
        } else {
            ""
        };
        format!("rounded-md border-2 transition {style} {tone} {ring}")
    };

    view! {
        // While searching or filtering by user, a box with nothing to show
        // steps aside.
        <Show when=move || { !ctx.narrowed() || shown.get() > 0 }>
            <section class=frame on:dragover=on_dragover on:drop=on_drop>
                <button
                    class="flex w-full items-center gap-2 rounded px-2 py-1.5 text-left hover:bg-slate-300/60 dark:hover:bg-slate-700/60"
                    aria-label=move || {
                        let kind = if is_sprint() { "sprint" } else { "epic" };
                        ids.with_value(|(_, name)| format!("Open {kind} {name}"))
                    }
                    aria-pressed=move || is_open().to_string()
                    title=move || {
                        if is_open() { "Close it".to_string() } else { "Show its tickets".to_string() }
                    }
                    on:click=move |_| {
                        let me = ids.get_value();
                        ctx.open_epic.update(|o| *o = if o.as_ref() == Some(&me) { None } else { Some(me) });
                    }
                >
                    <span class=format!(
                        "rounded px-1.5 text-[10px] font-semibold uppercase tracking-wide text-white {badge}",
                    )>{move || if is_sprint() { "Sprint" } else { "Epic" }}</span>
                    <span class="min-w-0 flex-1 truncate text-sm font-semibold" title=name.clone()>
                        {name.clone()}
                    </span>
                    <span class="whitespace-nowrap rounded-full bg-slate-300 px-2 text-xs dark:bg-slate-700">{amount}</span>
                </button>
                {move || {
                    refusal()
                        .map(|why| {
                            view! {
                                <p role="status" class="px-2 pb-1.5 text-xs font-medium text-red-700 dark:text-red-300">
                                    {why}
                                </p>
                            }
                        })
                }}
            </section>
        </Show>
    }
}

/// The epic or sprint opened from the Backlogs column, as a column beside
/// it (ticket 109): its tickets, which drag out to the backlog (or on, to
/// todo) like any card, and a drop target for backlog tickets. A sprint
/// also shows its length, state and points, and can be started here. It
/// closes with its × button, or by clicking its box again; it closes by
/// itself if the group is gone.
#[component]
fn OpenEpicColumn(repo: String, name: String) -> impl IntoView {
    let ctx = Ctx::get();
    let colour = epics_in_view(ctx)
        .into_iter()
        .find(|(r, n, _)| *r == repo && *n == name)
        .map_or(0, |(_, _, c)| c);
    let (tone, badge) = EPIC_COLOURS[colour % EPIC_COLOURS.len()];
    let here = StoredValue::new(Place::epic(&name).in_repo(&repo));
    let ids = StoredValue::new((repo.clone(), name.clone()));
    let group = group_memo(ctx, repo, name.clone());
    Effect::new(move |_| {
        if ctx.board.with(Option::is_some) && group.with(Option::is_none) {
            ctx.open_epic.set(None);
        }
    });
    let is_sprint = move || group.with(|g| g.as_ref().is_some_and(|g| g.kind == GroupKind::Sprint));
    let started = move || group.with(|g| g.as_ref().is_some_and(|g| g.started));
    let tickets = Memo::new(move |_| {
        let list = group.with(|g| g.as_ref().map(|g| g.tickets.clone()).unwrap_or_default());
        ctx.shown(list)
    });
    let count = move || tickets.with(Vec::len);
    let summary = move || {
        group.with(|g| {
            let g = g.as_ref()?;
            (g.kind == GroupKind::Sprint).then(|| {
                let state = if g.started { "started" } else { "pending" };
                format!("{} days · {state} · {} pts", g.days, total_points(g))
            })
        })
    };
    let refusal = move || here.with_value(|to| refusal(ctx, to));
    let (on_dragover, on_drop) = drop_target(ctx, here);
    let frame = move || {
        let style = if is_sprint() {
            "border-solid"
        } else {
            "border-dashed"
        };
        let target = if here.with_value(|to| accepts(ctx, to)) {
            "ring-2 ring-accent-500 ring-offset-1 ring-offset-slate-200 dark:ring-offset-slate-800"
        } else {
            ""
        };
        format!(
            "flex min-h-0 min-w-48 flex-1 basis-0 flex-col rounded-lg border-2 bg-slate-200/70 transition dark:bg-slate-800/70 {style} {tone} {target}"
        )
    };

    // Starting asks once more: there is no way back.
    let confirm_start = RwSignal::new(false);
    let start = move |_| {
        if !confirm_start.get_untracked() {
            confirm_start.set(true);
            return;
        }
        confirm_start.set(false);
        let (repo, name) = ids.get_value();
        spawn_local(async move {
            match api::start_sprint(Some(&repo), &name).await {
                Ok(report) => ctx.report(report),
                Err(e) => ctx.error.set(Some(e)),
            }
            ctx.reload();
        });
    };

    view! {
        <section
            aria-label=move || {
                let kind = if is_sprint() { "Sprint" } else { "Epic" };
                ids.with_value(|(_, name)| format!("{kind}: {name}"))
            }
            class=frame
            on:dragover=on_dragover
            on:drop=on_drop
        >
            <header class="flex items-center gap-2 px-3 py-2">
                <span class=format!(
                    "rounded px-1.5 text-[10px] font-semibold uppercase tracking-wide text-white {badge}",
                )>{move || if is_sprint() { "Sprint" } else { "Epic" }}</span>
                <h2 class="min-w-0 flex-1 truncate text-sm font-semibold" title=name.clone()>
                    {name.clone()}
                </h2>
                <span class="rounded-full bg-slate-300 px-2 text-xs dark:bg-slate-700">{count}</span>
                <button
                    class="rounded px-1.5 text-slate-500 hover:bg-slate-300 dark:hover:bg-slate-700"
                    title="Close"
                    aria-label=format!("Close {name}")
                    on:click=move |_| ctx.open_epic.set(None)
                >
                    "×"
                </button>
            </header>
            <div class="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto px-2 pb-2">
                {move || summary().map(|text| view! { <p class="px-1 text-[11px] text-slate-500">{text}</p> })}
                <Show when=move || is_sprint() && !started()>
                    <button
                        class=move || {
                            if confirm_start.get() {
                                "w-full rounded bg-accent-600 px-2 py-1 text-xs font-medium text-white hover:bg-accent-500"
                            } else {
                                "w-full rounded border border-slate-300 px-2 py-1 text-xs hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                            }
                        }
                        title="Once started, a sprint takes no more tickets and gives none back"
                        on:click=start
                        on:blur=move |_| confirm_start.set(false)
                    >
                        {move || if confirm_start.get() { "Confirm: start sprint" } else { "Start sprint" }}
                    </button>
                </Show>
                {move || {
                    refusal()
                        .map(|why| {
                            view! {
                                <p role="status" class="px-1 text-xs font-medium text-red-700 dark:text-red-300">
                                    {why}
                                </p>
                            }
                        })
                }}
                <ul class="flex flex-col gap-2">
                    <For each=move || tickets.get() key=|t| t.clone() children=|ticket| view! { <Card ticket /> } />
                </ul>
                <Show when=move || count() == 0>
                    <p class="px-1 py-2 text-center text-xs text-slate-500">
                        {move || {
                            if is_sprint() { "Drag sized backlog tickets here" } else { "Drag backlog tickets here" }
                        }}
                    </p>
                </Show>
            </div>
        </section>
    }
}

#[component]
fn Card(ticket: TicketSummary) -> impl IntoView {
    let ctx = Ctx::get();
    let TicketSummary {
        file,
        column,
        id,
        title,
        assignee,
        tags,
        priority,
        epic,
        repo,
        repository,
        epic_label,
        points,
        lock,
        lapsed,
        ..
    } = ticket;
    // Someone else's lock: the card can be opened to read, but not dragged.
    let held = lock.as_ref().is_some_and(|l| !l.mine);
    // A draft (ticket 114) says so first, and its outline is dashed.
    let draft = epic::is_draft(&tags);
    // Outside its epic's box, a card still says which epic it belongs to.
    let epic_tag = (epic.is_none() && !epic_label.is_empty()).then(|| {
        let colour = epics_in_view(ctx)
            .into_iter()
            .find(|(r, n, _)| *r == repo && *n == epic_label)
            .map(|(_, _, c)| c);
        let tone = colour.map_or("bg-slate-500", |c| EPIC_COLOURS[c % EPIC_COLOURS.len()].1);
        (epic_label.clone(), tone)
    });
    let place = StoredValue::new(Place {
        repo: repo.clone(),
        column,
        epic,
    });
    let drag_file = file.clone();
    let is_dragging = {
        let file = file.clone();
        move || {
            ctx.dragging
                .with(|d| d.as_ref().is_some_and(|(f, _)| *f == file))
        }
    };

    view! {
        <li
            draggable=if held { "false" } else { "true" }
            tabindex="0"
            class=move || {
                format!(
                    "{} rounded-md border border-slate-200 bg-white p-2.5 text-sm shadow-sm hover:border-accent-400 focus:outline-2 focus:outline-accent-500 dark:border-slate-700 dark:bg-slate-900 {} {}",
                    if held { "cursor-pointer" } else { "cursor-grab" },
                    if is_dragging() { "opacity-50" } else { "" },
                    if draft { "border-dashed border-amber-400 dark:border-amber-500" } else { "" },
                )
            }
            on:dragstart=move |ev: DragEvent| {
                // WebKit only starts a drag once some data has been set.
                if let Some(dt) = ev.data_transfer() {
                    let _ = dt.set_data("text/plain", &drag_file);
                    dt.set_effect_allowed("move");
                }
                ctx.dragging.set(Some((drag_file.clone(), place.get_value())));
            }
            on:dragend=move |_| ctx.dragging.set(None)
            on:click={
                let file = file.clone();
                move |_| ctx.open_ticket(place.get_value(), file.clone())
            }
            on:keydown=move |ev| {
                if ev.key() == "Enter" {
                    ctx.open_ticket(place.get_value(), file.clone());
                }
            }
        >
            <div class="flex items-baseline gap-2">
                <span class="font-mono text-xs text-slate-500">{id}</span>
                <span class="flex-1 font-medium">{title}</span>
            </div>
            // The repository it's about (ticket 101), unless the filter
            // already says so.
            {(!repository.is_empty())
                .then(|| {
                    let name = repository.clone();
                    view! {
                        <Show when=move || ctx.about.with(Option::is_none)>
                            <div class="mt-0.5 truncate text-[11px] text-slate-500" title="Repository">
                                {name.clone()}
                            </div>
                        </Show>
                    }
                })}
            <div class="mt-1.5 flex flex-wrap items-center gap-1 text-xs">
                {draft
                    .then(|| {
                        view! {
                            <span
                                class="rounded border border-dashed border-amber-500 px-1.5 font-semibold text-amber-700 dark:text-amber-300"
                                title="Draft: stays in the backlog until DRAFT is removed"
                            >
                                "DRAFT"
                            </span>
                        }
                    })}
                {match (lock, lapsed) {
                    (Some(l), _) => Some(view! { <LockBadge lock=l lapsed=false /> }),
                    (None, Some(l)) => Some(view! { <LockBadge lock=l lapsed=true /> }),
                    (None, None) => None,
                }}
                {epic_tag
                    .map(|(name, tone)| {
                        view! {
                            <span
                                class=format!("max-w-40 truncate rounded px-1.5 font-medium text-white {tone}")
                                title=format!("Epic: {name}")
                            >
                                {name.clone()}
                            </span>
                        }
                    })}
                {(!points.trim().is_empty())
                    .then(|| {
                        view! {
                            <span
                                class="rounded bg-slate-200 px-1.5 font-mono text-slate-700 dark:bg-slate-700 dark:text-slate-200"
                                title="Points"
                            >
                                {format!("{} pts", points.trim())}
                            </span>
                        }
                    })}
                {(!priority.is_empty())
                    .then(|| {
                        view! {
                            <span
                                class=format!("rounded px-1.5 font-medium {}", priority_tone(&priority))
                                title="Priority"
                            >
                                {priority.clone()}
                            </span>
                        }
                    })}
                {(!assignee.is_empty())
                    .then(|| {
                        view! { <span class="text-slate-500">"@" {assignee}</span> }
                    })}
                {tags
                    .into_iter()
                    .filter(|tag| !tag.trim().eq_ignore_ascii_case(epic::DRAFT_TAG))
                    .map(|tag| {
                        view! {
                            <span class="rounded bg-accent-100 px-1.5 uppercase text-accent-800 dark:bg-accent-900 dark:text-accent-200">
                                {tag}
                            </span>
                        }
                    })
                    .collect_view()}
            </div>
        </li>
    }
}

/// A lock on a card: whose, and until when on hover. Someone else's stands
/// out; your own is quieter. A lapsed one (ticket 110) is outlined and faint:
/// still recorded, but it holds nothing.
#[component]
pub fn LockBadge(lock: amd_ui_model::Lock, lapsed: bool) -> impl IntoView {
    let who = if lock.mine {
        "you".to_string()
    } else {
        lock.by.split('@').next().unwrap_or(&lock.by).to_string()
    };
    let tone = if lapsed {
        "border border-dashed border-slate-400 text-slate-500 dark:border-slate-500 dark:text-slate-400"
    } else if lock.mine {
        "bg-slate-200 text-slate-700 dark:bg-slate-700 dark:text-slate-200"
    } else {
        "bg-amber-200 text-amber-900 dark:bg-amber-900 dark:text-amber-100"
    };
    let title = if lapsed && lock.mine {
        format!(
            "Your lock lapsed at {}: lock it again to keep working",
            lock.until_local
        )
    } else if lapsed {
        format!(
            "Locked by {} until {} (lapsed): anyone can change it now",
            lock.by, lock.until_local
        )
    } else if lock.mine {
        format!("Locked by you until {}", lock.until_local)
    } else {
        format!(
            "Locked by {} until {}: only they can change it",
            lock.by, lock.until_local
        )
    };
    view! {
        <span class=format!("inline-flex items-center gap-1 rounded px-1.5 font-medium {tone}") title=title>
            <svg viewBox="0 0 16 16" class="h-3 w-3" fill="currentColor" aria-hidden="true">
                <path d="M4 7V5a4 4 0 1 1 8 0v2h.5A1.5 1.5 0 0 1 14 8.5v5a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 13.5v-5A1.5 1.5 0 0 1 3.5 7H4Zm2 0h4V5a2 2 0 1 0-4 0v2Z" />
            </svg>
            {who}
        </span>
    }
}

/// The containers shown in the Backlogs column — sprints first, then epics —
/// as (repository, name, colour). The same assignment colours the tags on
/// cards outside the column.
fn epics_in_view(ctx: Ctx) -> Vec<(String, String, usize)> {
    let wanted = ctx.repo.get();
    ctx.board.with(|b| {
        let mut groups: Vec<(bool, String, String)> = b
            .as_ref()
            .map(|b| {
                b.epics
                    .iter()
                    .filter(|e| wanted.as_ref().is_none_or(|r| *r == e.repo))
                    .map(|e| (e.kind != GroupKind::Sprint, e.repo.clone(), e.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        groups.sort();
        let shown: Vec<(String, String)> = groups.into_iter().map(|(_, r, n)| (r, n)).collect();
        let names: Vec<String> = shown.iter().map(|(_, n)| n.clone()).collect();
        let colours = epic::colours(&names, EPIC_COLOURS.len());
        shown
            .into_iter()
            .zip(colours)
            .map(|((repo, name), colour)| (repo, name, colour))
            .collect()
    })
}

/// Each epic's border and badge colours, chosen by `epic::colours` so that
/// neighbouring epics never match. Written out in full for Tailwind.
const EPIC_COLOURS: [(&str, &str); 6] = [
    (
        "border-violet-500 bg-violet-50/70 dark:bg-violet-950/40",
        "bg-violet-600",
    ),
    (
        "border-teal-500 bg-teal-50/70 dark:bg-teal-950/40",
        "bg-teal-600",
    ),
    (
        "border-amber-500 bg-amber-50/70 dark:bg-amber-950/40",
        "bg-amber-600",
    ),
    (
        "border-rose-500 bg-rose-50/70 dark:bg-rose-950/40",
        "bg-rose-600",
    ),
    (
        "border-sky-500 bg-sky-50/70 dark:bg-sky-950/40",
        "bg-sky-600",
    ),
    (
        "border-lime-600 bg-lime-50/70 dark:bg-lime-950/40",
        "bg-lime-700",
    ),
];

/// Badge colours, quietest for low; an unknown value is shown in red so it
/// stands out as needing correction.
fn priority_tone(priority: &str) -> &'static str {
    match priority {
        "low" => "bg-slate-200 text-slate-700 dark:bg-slate-700 dark:text-slate-200",
        "medium" => "bg-sky-100 text-sky-800 dark:bg-sky-900 dark:text-sky-200",
        "high" => "bg-amber-100 text-amber-900 dark:bg-amber-900 dark:text-amber-100",
        _ => "border border-red-500 text-red-700 dark:text-red-300",
    }
}
