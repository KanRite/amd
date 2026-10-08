//! The admin panel: which tasks directory the board shows, and the template
//! that defines the standard ticket format.

use amd_ui_model::TicketTemplate;
use amd_ui_model::wip::WipLimits;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::app::{Ctx, Modal};

#[component]
pub fn AdminPanel() -> impl IntoView {
    let ctx = Ctx::get();
    let dir = RwSignal::new(String::new());
    let template = RwSignal::new(None::<TicketTemplate>);
    let source = RwSignal::new(String::new());
    let preview = RwSignal::new(None::<Result<String, String>>);
    let auto_commit = RwSignal::new(true);
    spawn_local(async move {
        if let Ok(on) = api::auto_commit().await {
            auto_commit.set(on);
        }
    });
    let status = RwSignal::new(None::<String>);

    let show_template = move |t: TicketTemplate| {
        source.set(t.source.clone());
        template.set(Some(t));
        preview.set(None);
    };
    let load_template = move || {
        spawn_local(async move {
            match api::get_template().await {
                Ok(t) => show_template(t),
                Err(e) => status.set(Some(e)),
            }
        });
    };

    spawn_local(async move {
        if let Ok(Some(d)) = api::tasks_dir().await {
            dir.set(d);
        }
    });
    load_template();

    let open_dir = move |_| {
        let path = dir.get_untracked();
        spawn_local(async move {
            match api::set_tasks_dir(&path).await {
                Ok(board) => {
                    dir.set(board.tasks_dir.clone());
                    ctx.board.set(Some(board));
                    ctx.error.set(None);
                    status.set(Some("Board opened.".into()));
                    load_template();
                }
                Err(e) => status.set(Some(e)),
            }
        });
    };
    let run_preview = move |_| {
        let src = source.get_untracked();
        spawn_local(async move { preview.set(Some(api::preview_template(&src).await)) });
    };
    let save = move |_| {
        let src = source.get_untracked();
        spawn_local(async move {
            match api::save_template(&src).await {
                Ok(t) => {
                    show_template(t);
                    status.set(Some("Template saved.".into()));
                }
                Err(e) => preview.set(Some(Err(e))),
            }
        });
    };
    let reset = move |_| {
        spawn_local(async move {
            match api::reset_template().await {
                Ok(t) => {
                    show_template(t);
                    status.set(Some("Using the built-in template.".into()));
                }
                Err(e) => status.set(Some(e)),
            }
        });
    };

    let dirty =
        move || template.with(|t| t.as_ref().is_some_and(|t| source.with(|s| *s != t.source)));
    let customised = move || template.with(|t| t.as_ref().is_some_and(|t| t.customised));
    let template_note = move || {
        template.with(|t| match t {
            Some(t) if t.customised => format!("Customised for this board: {}", t.path),
            Some(t) => format!("Built-in default. Saving creates {}", t.path),
            None => "Open a board to edit its template.".into(),
        })
    };

    let field = "rounded border border-slate-300 bg-slate-50 px-2 py-1.5 dark:border-slate-600 dark:bg-slate-900";
    let secondary = "rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700";
    let primary = "rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50";

    view! {
        <Modal title="Admin".to_string() width="max-w-5xl">
            <div class="flex min-h-0 flex-col gap-4 overflow-y-auto">
                <section class="flex flex-col gap-2">
                    <h3 class="text-sm font-semibold">"Tasks directory"</h3>
                    <div class="flex gap-2">
                        <input
                            type="text"
                            aria-label="Tasks directory"
                            placeholder="/path/to/project-tickets/tasks"
                            class=format!("{field} flex-1 font-mono text-sm")
                            prop:value=move || dir.get()
                            on:input=move |ev| dir.set(event_target_value(&ev))
                        />
                        <button class=secondary on:click=open_dir>
                            "Open"
                        </button>
                    </div>
                    <p class="text-xs text-slate-500">
                        "A directory holding backlog/, todo/, doing/ and done/. It is remembered for next time."
                    </p>
                </section>

                <section class="flex flex-col gap-1">
                    <label class="flex items-center gap-2 text-sm font-semibold">
                        <input
                            type="checkbox"
                            prop:checked=move || auto_commit.get()
                            on:change=move |ev| {
                                let on = event_target_checked(&ev);
                                spawn_local(async move {
                                    match api::set_auto_commit(on).await {
                                        Ok(on) => auto_commit.set(on),
                                        Err(e) => status.set(Some(e)),
                                    }
                                });
                            }
                        />
                        "Commit and push ticket changes"
                    </label>
                    <p class="text-xs text-slate-500">
                        "Creating, editing, moving or archiving a ticket commits just that change and pushes it to main, as amd does. Boards outside a git repository are never committed."
                    </p>
                </section>

                <TagsAdmin />

                <WipAdmin />

                <LeaseAdmin />

                <AppearanceAdmin />

                <EpicsAdmin />

                <section class="flex flex-col gap-2">
                    <h3 class="text-sm font-semibold">"Ticket template"</h3>
                    <p class="text-xs text-slate-500">{template_note}</p>
                    <div class="grid min-h-0 grid-cols-1 gap-3 md:grid-cols-2">
                        <textarea
                            aria-label="Ticket template source"
                            spellcheck="false"
                            class=format!("{field} h-[45vh] resize-none font-mono text-xs")
                            prop:value=move || source.get()
                            on:input=move |ev| source.set(event_target_value(&ev))
                        ></textarea>
                        <pre
                            aria-label="Template preview"
                            class=move || {
                                let tone = if matches!(preview.get(), Some(Err(_))) {
                                    "text-red-700 dark:text-red-300"
                                } else {
                                    "text-slate-700 dark:text-slate-300"
                                };
                                format!(
                                    "h-[45vh] overflow-auto whitespace-pre-wrap rounded bg-slate-100 p-2 font-mono text-xs dark:bg-slate-900 {tone}",
                                )
                            }
                        >
                            {move || match preview.get() {
                                Some(Ok(text)) | Some(Err(text)) => text,
                                None => "Preview shows a new ticket rendered from this template.".into(),
                            }}
                        </pre>
                    </div>
                    <div class="flex items-center gap-2">
                        <span class="flex-1 text-xs text-slate-500">
                            {move || status.get().unwrap_or_default()}
                        </span>
                        <button class=secondary disabled=move || !customised() on:click=reset>
                            "Reset to built-in"
                        </button>
                        <button class=secondary disabled=move || template.with(Option::is_none) on:click=run_preview>
                            "Preview"
                        </button>
                        <button class=primary disabled=move || !dirty() on:click=save>
                            "Save template"
                        </button>
                    </div>
                </section>
            </div>
        </Modal>
    }
}

/// The backlog's epics, each renamable. A rename moves the epic's folder and
/// relabels its tickets in one commit (see `TasksDir::rename_epic`).
#[component]
fn EpicsAdmin() -> impl IntoView {
    let ctx = Ctx::get();
    let epics = move || {
        ctx.board.with(|b| {
            b.as_ref()
                .map(|b| {
                    b.epics
                        .iter()
                        .map(|e| (e.repo.clone(), e.name.clone(), e.tickets.len()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
    };
    view! {
        <section class="flex flex-col gap-2">
            <h3 class="text-sm font-semibold">"Sprints and epics"</h3>
            <Show
                when=move || !epics().is_empty()
                fallback=|| {
                    view! {
                        <p class="text-xs text-slate-500">
                            "None yet. Create one with \"+ Sprint\" or \"+ Epic\" in the Backlogs column."
                        </p>
                    }
                }
            >
                <ul class="flex flex-col gap-2">
                    <For
                        each=epics
                        key=|row| row.clone()
                        children=|(repo, name, count)| view! { <EpicRow repo name count /> }
                    />
                </ul>
            </Show>
        </section>
    }
}

#[component]
fn EpicRow(repo: String, name: String, count: usize) -> impl IntoView {
    let repo = StoredValue::new(repo);
    let ctx = Ctx::get();
    let current = StoredValue::new(name.clone());
    let draft = RwSignal::new(name.clone());
    let problem = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let changed = move || current.with_value(|c| draft.with(|d| d.trim() != c));

    let rename = move || {
        if busy.get_untracked() || !changed() {
            return;
        }
        let (from, to, repo) = (current.get_value(), draft.get_untracked(), repo.get_value());
        busy.set(true);
        spawn_local(async move {
            match api::rename_epic(Some(&repo), &from, &to).await {
                Ok(report) => {
                    ctx.report(report);
                    ctx.reload();
                }
                Err(e) => problem.set(Some(e)),
            }
            busy.set(false);
        });
    };

    view! {
        <li class="flex flex-col gap-1">
            <form
                class="flex items-center gap-2"
                on:submit=move |ev| {
                    ev.prevent_default();
                    rename();
                }
            >
                <input
                    type="text"
                    aria-label=format!("Name of the {name} epic")
                    aria-invalid=move || problem.with(Option::is_some).to_string()
                    class="flex-1 rounded border border-slate-300 bg-slate-50 px-2 py-1.5 text-sm dark:border-slate-600 dark:bg-slate-900"
                    prop:value=move || draft.get()
                    on:input=move |ev| {
                        draft.set(event_target_value(&ev));
                        problem.set(None);
                    }
                />
                <Show when=move || ctx.multi_repo()>
                    <span class="max-w-40 shrink-0 truncate text-xs text-slate-500" title="Repository">
                        {repo.get_value()}
                    </span>
                </Show>
                <span class="w-20 shrink-0 text-right text-xs text-slate-500">
                    {format!("{count} ticket{}", if count == 1 { "" } else { "s" })}
                </span>
                <button
                    type="submit"
                    class="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700"
                    disabled=move || busy.get() || !changed()
                >
                    "Rename"
                </button>
            </form>
            {move || {
                problem
                    .get()
                    .map(|p| {
                        view! {
                            <p role="alert" class="text-xs font-medium text-red-700 dark:text-red-300">
                                {p}
                            </p>
                        }
                    })
            }}
        </li>
    }
}

/// Your colour scheme: light, dark or follow the system, and an accent
/// colour. Applied at once and saved in your own settings on this machine.
#[component]
fn AppearanceAdmin() -> impl IntoView {
    use amd_ui_model::appearance::{Accent, Appearance, Theme};
    let ctx = Ctx::get();
    let problem = RwSignal::new(None::<String>);
    let choose = move |change: Appearance| {
        ctx.appearance.set(change);
        spawn_local(async move {
            match api::set_appearance(change).await {
                Ok(_) => problem.set(None),
                Err(e) => problem.set(Some(e)),
            }
        });
    };
    // Written out in full so Tailwind generates them.
    let swatch = |accent: Accent| match accent {
        Accent::Indigo => "bg-indigo-600",
        Accent::Teal => "bg-teal-600",
        Accent::Rose => "bg-rose-600",
        Accent::Amber => "bg-amber-600",
        Accent::Slate => "bg-slate-600",
    };
    view! {
        <section class="flex flex-col gap-2">
            <h3 class="text-sm font-semibold">"Appearance"</h3>
            <div role="radiogroup" aria-label="Theme" class="flex flex-wrap gap-2">
                {Theme::ALL
                    .map(|theme| {
                        let on = move || ctx.appearance.with(|a| a.theme == theme);
                        view! {
                            <button
                                type="button"
                                role="radio"
                                aria-checked=move || on().to_string()
                                class=move || {
                                    if on() {
                                        "rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white"
                                    } else {
                                        "rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                                    }
                                }
                                on:click=move |_| {
                                    choose(Appearance { theme, ..ctx.appearance.get_untracked() })
                                }
                            >
                                {theme.label()}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
            <div role="radiogroup" aria-label="Accent colour" class="flex flex-wrap items-center gap-3">
                <span class="text-xs text-slate-500">"Accent"</span>
                {Accent::ALL
                    .map(|accent| {
                        let on = move || ctx.appearance.with(|a| a.accent == accent);
                        view! {
                            <button
                                type="button"
                                role="radio"
                                aria-checked=move || on().to_string()
                                aria-label=accent.label()
                                title=accent.label()
                                class=move || {
                                    format!(
                                        "h-7 w-7 rounded-full {} {}",
                                        swatch(accent),
                                        if on() {
                                            "ring-2 ring-offset-2 ring-slate-900 ring-offset-white dark:ring-white dark:ring-offset-slate-800"
                                        } else {
                                            "opacity-80 hover:opacity-100"
                                        },
                                    )
                                }
                                on:click=move |_| {
                                    choose(Appearance { accent, ..ctx.appearance.get_untracked() })
                                }
                            ></button>
                        }
                    })
                    .collect_view()}
            </div>
            <p class="text-xs text-slate-500">
                "Saved in your own settings on this machine, and used again next time."
            </p>
            {move || {
                problem
                    .get()
                    .map(|p| {
                        view! {
                            <p role="alert" class="text-xs font-medium text-red-700 dark:text-red-300">
                                {p}
                            </p>
                        }
                    })
            }}
        </section>
    }
}

/// The board's WIP limit on doing: kept with the tickets in `.settings`,
/// so the whole team works to the same one.
#[component]
fn WipAdmin() -> impl IntoView {
    let ctx = Ctx::get();
    // The board's own repository, which a change is saved to.
    let current = move || {
        ctx.board.with(|b| {
            b.as_ref()
                .and_then(|b| b.repos.first())
                .map(|r| (r.name.clone(), r.wip))
        })
    };
    let initial = current().map(|(_, w)| w).unwrap_or_default();
    let doing = RwSignal::new(initial.doing.to_string());
    let note = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let save = move |_| {
        let Some(d) = doing
            .get_untracked()
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|n| *n <= 99)
        else {
            note.set(Some(
                "Give the limit as a whole number from 0 to 99.".into(),
            ));
            return;
        };
        let repo = current().map(|(name, _)| name);
        busy.set(true);
        spawn_local(async move {
            match api::set_wip_limits(WipLimits { doing: d }, repo.as_deref()).await {
                Ok(report) => {
                    ctx.report(report);
                    note.set(Some("Limit saved.".into()));
                    ctx.reload();
                }
                Err(e) => note.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let changed =
        move || current().is_some_and(|(_, w)| doing.with(|d| d.trim() != w.doing.to_string()));

    view! {
        <section class="flex flex-col gap-2">
            <h3 class="text-sm font-semibold">"WIP limit"</h3>
            <div class="flex flex-wrap items-center gap-4">
                <label class="flex items-center gap-2 text-sm">
                    "Doing"
                    <input
                        type="number"
                        min="0"
                        max="99"
                        class="w-20 rounded border border-slate-300 bg-slate-50 px-2 py-1 dark:border-slate-600 dark:bg-slate-900"
                        aria-label="Doing limit"
                        prop:value=move || doing.get()
                        on:input=move |ev| {
                            doing.set(event_target_value(&ev));
                            note.set(None);
                        }
                    />
                </label>
                <button
                    class="rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50"
                    disabled=move || busy.get() || !changed()
                    on:click=save
                >
                    "Save limit"
                </button>
                {move || note.get().map(|n| view! { <span class="text-sm">{n}</span> })}
            </div>
            <p class="text-xs text-slate-500">
                {format!(
                    "The most tickets Doing may hold at once ({} unless changed); 0 means no limit. To do has no limit, so work never gets stuck. A move into a full Doing asks before going ahead, and moving a ticket out is never blocked. Saved with the tickets, in .settings, and committed and pushed.",
                    amd_ui_model::wip::DEFAULT_LIMIT,
                )}
            </p>
        </section>
    }
}

/// The project's tag list (ticket 122): the listed tags with how many
/// tickets carry each, a form to add one (saved in UPPERCASE), a dropdown of
/// the unused ones to remove, and whether only listed tags may be used.
/// Kept with the tickets in `.settings`, like the WIP limit.
#[component]
fn TagsAdmin() -> impl IntoView {
    let ctx = Ctx::get();
    let tags = move || {
        ctx.board
            .with(|b| b.as_ref().map(|b| b.tags.clone()).unwrap_or_default())
    };
    let only = move || ctx.board.with(|b| b.as_ref().is_some_and(|b| b.tags_only));
    let listed = move || tags().into_iter().filter(|t| t.listed).collect::<Vec<_>>();
    let unused = move || {
        listed()
            .into_iter()
            .filter(|t| t.count == 0)
            .map(|t| t.name)
            .collect::<Vec<_>>()
    };
    let unlisted = move || {
        tags()
            .into_iter()
            .filter(|t| !t.listed && !t.reserved)
            .map(|t| format!("{} ({})", t.name, t.count))
            .collect::<Vec<_>>()
    };
    let new_tag = RwSignal::new(String::new());
    let to_remove = RwSignal::new(String::new());
    let note = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let add = move || {
        let tag = new_tag.get_untracked().trim().to_uppercase();
        if tag.is_empty() {
            note.set(Some("Type a tag first.".into()));
            return;
        }
        busy.set(true);
        spawn_local(async move {
            match api::add_tag(&tag).await {
                Ok(report) => {
                    ctx.report(report);
                    note.set(Some(format!("Added {tag}.")));
                    new_tag.set(String::new());
                    ctx.reload();
                }
                Err(e) => note.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let remove = move |_| {
        let tag = to_remove.get_untracked();
        if tag.is_empty() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            match api::remove_tag(&tag).await {
                Ok(report) => {
                    ctx.report(report);
                    note.set(Some(format!("Removed {tag}.")));
                    to_remove.set(String::new());
                    ctx.reload();
                }
                Err(e) => note.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let set_only = move |on: bool| {
        busy.set(true);
        spawn_local(async move {
            match api::set_tags_only(on).await {
                Ok(report) => {
                    ctx.report(report);
                    ctx.reload();
                }
                Err(e) => note.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let field = "rounded border border-slate-300 bg-slate-50 px-2 py-1 text-sm dark:border-slate-600 dark:bg-slate-900";
    let button = "rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50";

    view! {
        <section class="flex flex-col gap-2">
            <h3 class="text-sm font-semibold">"Tags"</h3>
            {move || {
                let all = listed();
                if all.is_empty() {
                    view! {
                        <p class="text-sm text-slate-500">
                            "No tags defined yet, so any tag can be used. Adding one starts the project's list."
                        </p>
                    }
                        .into_any()
                } else {
                    view! {
                        <ul class="flex flex-wrap gap-1.5" aria-label="Listed tags">
                            {all
                                .into_iter()
                                .map(|t| {
                                    let label = format!("{} · {}", t.name, t.count);
                                    view! {
                                        <li class="rounded bg-accent-100 px-1.5 text-sm text-accent-800 dark:bg-accent-900 dark:text-accent-200">
                                            {label}
                                        </li>
                                    }
                                })
                                .collect_view()}
                        </ul>
                    }
                        .into_any()
                }
            }}
            <div class="flex flex-wrap items-center gap-2">
                <input
                    type="text"
                    class=format!("{field} w-48 uppercase")
                    aria-label="New tag"
                    placeholder="New tag"
                    prop:value=move || new_tag.get()
                    on:input=move |ev| {
                        new_tag.set(event_target_value(&ev));
                        note.set(None);
                    }
                    on:keydown=move |ev| {
                        if ev.key() == "Enter" {
                            ev.prevent_default();
                            add();
                        }
                    }
                />
                <button
                    class=button
                    disabled=move || busy.get() || new_tag.with(|t| t.trim().is_empty())
                    on:click=move |_| add()
                >
                    "Add tag"
                </button>
            </div>
            <div class="flex flex-wrap items-center gap-2">
                <select
                    class=format!("{field} w-48 appearance-none")
                    aria-label="Unused tag to remove"
                    disabled=move || unused().is_empty()
                    prop:value=move || to_remove.get()
                    on:change=move |ev| to_remove.set(event_target_value(&ev))
                >
                    <option value="">
                        {move || {
                            if unused().is_empty() { "No unused tags" } else { "Choose an unused tag" }
                        }}
                    </option>
                    {move || {
                        unused()
                            .into_iter()
                            .map(|name| {
                                let value = name.clone();
                                view! { <option value=value>{name}</option> }
                            })
                            .collect_view()
                    }}
                </select>
                <button
                    class=button
                    disabled=move || busy.get() || to_remove.with(|t| t.is_empty())
                    on:click=remove
                >
                    "Remove"
                </button>
            </div>
            <label class="flex items-center gap-2 text-sm">
                <input
                    type="checkbox"
                    prop:checked=only
                    disabled=move || busy.get()
                    on:change=move |ev| set_only(event_target_checked(&ev))
                />
                "Prevent arbitrary tags"
            </label>
            {move || note.get().map(|n| view! { <p class="text-sm">{n}</p> })}
            {move || {
                let others = unlisted();
                (!others.is_empty())
                    .then(|| {
                        view! {
                            <p class="text-xs text-slate-500">
                                "In use but not listed: " {others.join(", ")}
                            </p>
                        }
                    })
            }}
            {move || {
                let built_in = tags()
                    .into_iter()
                    .filter(|t| t.reserved)
                    .map(|t| format!("{} ({})", t.name, t.count))
                    .collect::<Vec<_>>();
                (!built_in.is_empty())
                    .then(|| {
                        view! {
                            <p class="text-xs text-slate-500">
                                "Built in: " {built_in.join(", ")}
                                ". DRAFT keeps a ticket in the backlog until it's removed; it's always allowed and can't be removed."
                            </p>
                        }
                    })
            }}
            <p class="text-xs text-slate-500">
                "Tags are always UPPERCASE. A tag can only be removed once no ticket on the project's boards carries it. With \"Prevent arbitrary tags\" on (the default), only these tags can be used; off, any tag can be typed, and ones not on the list are flagged. With no list, any tag goes. Saved with the tickets, in .settings, and committed and pushed."
            </p>
        </section>
    }
}

/// How long a ticket lock lasts on the board: kept with the tickets in
/// `.settings` (`lease-hours`), a team decision like the WIP limit.
#[component]
fn LeaseAdmin() -> impl IntoView {
    let ctx = Ctx::get();
    let current = move || {
        ctx.board.with(|b| {
            b.as_ref()
                .and_then(|b| b.repos.first())
                .map(|r| (r.name.clone(), r.lease_hours))
        })
    };
    let initial = current().map_or(amd_ui_model::DEFAULT_LEASE_HOURS, |(_, h)| h);
    let hours = RwSignal::new(initial.to_string());
    let note = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let save = move |_| {
        let Some(h) = hours
            .get_untracked()
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|h| (1..=168).contains(h))
        else {
            note.set(Some("Give a whole number of hours from 1 to 168.".into()));
            return;
        };
        let repo = current().map(|(name, _)| name);
        busy.set(true);
        spawn_local(async move {
            match api::set_lease_hours(h, repo.as_deref()).await {
                Ok(report) => {
                    ctx.report(report);
                    note.set(Some("Saved.".into()));
                    ctx.reload();
                }
                Err(e) => note.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let changed = move || current().is_some_and(|(_, h)| hours.with(|v| v.trim() != h.to_string()));

    view! {
        <section class="flex flex-col gap-2">
            <h3 class="text-sm font-semibold">"Ticket locks"</h3>
            <div class="flex flex-wrap items-center gap-4">
                <label class="flex items-center gap-2 text-sm">
                    "A lock lasts"
                    <input
                        type="number"
                        min="1"
                        max="168"
                        class="w-20 rounded border border-slate-300 bg-slate-50 px-2 py-1 dark:border-slate-600 dark:bg-slate-900"
                        aria-label="Lock length in hours"
                        prop:value=move || hours.get()
                        on:input=move |ev| {
                            hours.set(event_target_value(&ev));
                            note.set(None);
                        }
                    />
                    "hours"
                </label>
                <button
                    class="rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50"
                    disabled=move || busy.get() || !changed()
                    on:click=save
                >
                    "Save"
                </button>
                {move || note.get().map(|n| view! { <span class="text-sm">{n}</span> })}
            </div>
            <p class="text-xs text-slate-500">
                {format!(
                    "Moving a ticket into To do or Doing locks it for you ({} hours unless changed); lock and unlock from a ticket's editor. While someone else's lock lasts, the board will not change their ticket. Saved with the tickets, in .settings, and committed and pushed.",
                    amd_ui_model::DEFAULT_LEASE_HOURS,
                )}
            </p>
        </section>
    }
}
