//! The Roadmaps view (ticket 206): one roadmap per board of the project,
//! shown as buttons in a grid with their duration and time remaining. Opening
//! one shows its goal, whether its milestones are sprints (epics otherwise)
//! and the milestones, each of which makes its epic or sprint on the board.

use amd_ui_model::{MilestoneCard, RoadmapCard, RoadmapsPage, days_label};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::app::Ctx;

const FIELD: &str = "rounded border border-slate-300 bg-slate-50 px-2 py-1.5 text-sm dark:border-slate-600 dark:bg-slate-900";
const SELECT: &str = "appearance-none rounded border border-slate-300 bg-slate-50 px-2 py-1.5 text-sm dark:border-slate-600 dark:bg-slate-900";
const PRIMARY: &str = "rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50";
const SECONDARY: &str = "rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-800";

const GREEN: &str = "border-green-600 bg-green-50 text-green-900 dark:border-green-500 dark:bg-green-950 dark:text-green-100";
const RED: &str =
    "border-red-600 bg-red-50 text-red-900 dark:border-red-500 dark:bg-red-950 dark:text-red-100";
const PLAIN: &str = "border-slate-300 dark:border-slate-600";

fn dated(label: &str, date: &str) -> String {
    if date.is_empty() {
        String::new()
    } else {
        format!(" · {label} {date}")
    }
}

/// "5 days left", or "2 days over".
fn left(remaining: i64) -> String {
    if remaining < 0 {
        format!("{} over", days_label(remaining))
    } else {
        format!("{} left", days_label(remaining))
    }
}

fn tone(roadmap: &RoadmapCard) -> &'static str {
    if roadmap.duration.is_none() && roadmap.remaining.is_none() {
        PLAIN
    } else if roadmap.late {
        RED
    } else {
        GREEN
    }
}

#[component]
pub fn RoadmapsView() -> impl IntoView {
    let page = RwSignal::new(None::<Result<RoadmapsPage, String>>);
    let open = RwSignal::new(None::<String>);
    let reload = move || spawn_local(async move { page.set(Some(api::list_roadmaps().await)) });
    reload();

    view! {
        <div class="mx-auto flex h-full max-w-5xl flex-col gap-4 overflow-y-auto p-4">
            {move || match page.get() {
                None => view! { <p class="text-sm text-slate-500">"Loading…"</p> }.into_any(),
                Some(Err(e)) => {
                    view! { <p role="alert" class="text-sm text-red-700 dark:text-red-300">{e}</p> }
                        .into_any()
                }
                Some(Ok(data)) => {
                    let opened = open
                        .get()
                        .and_then(|slug| data.roadmaps.iter().find(|r| r.slug == slug).cloned());
                    match opened {
                        Some(roadmap) => {
                            view! { <RoadmapView roadmap open reload /> }.into_any()
                        }
                        None => view! { <RoadmapList data open reload /> }.into_any(),
                    }
                }
            }}
        </div>
    }
}

#[component]
fn Problem(problem: RwSignal<Option<String>>) -> impl IntoView {
    move || {
        problem.get().map(|p| {
            view! { <p role="alert" class="text-sm font-medium text-red-700 dark:text-red-300">{p}</p> }
        })
    }
}

/// The roadmap buttons on the left, the new-roadmap form on the right.
#[component]
fn RoadmapList(
    data: RoadmapsPage,
    open: RwSignal<Option<String>>,
    reload: impl Fn() + Copy + 'static,
) -> impl IntoView {
    let ctx = Ctx::get();
    let busy = RwSignal::new(false);
    let problem = RwSignal::new(None::<String>);
    let board = RwSignal::new(data.free_boards.first().cloned().unwrap_or_default());
    let free = data.free_boards.clone();

    let create = move || {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            match api::create_roadmap(&board.get_untracked()).await {
                Ok(report) => {
                    ctx.report(report);
                    problem.set(None);
                    reload();
                }
                Err(e) => problem.set(Some(e)),
            }
            busy.set(false);
        });
    };

    let (planned, unassigned): (Vec<_>, Vec<_>) =
        data.roadmaps.into_iter().partition(|r| !r.board.is_empty());
    let none_yet = planned.is_empty();

    view! {
        <h1 class="text-xl font-semibold">"Roadmaps"</h1>
        <div class="flex flex-wrap items-start gap-6">
            <div class="min-w-0 flex-1">
                {none_yet
                    .then(|| {
                        view! { <p class="text-sm text-slate-500">"No roadmaps yet."</p> }
                    })}
                <div class="grid gap-3" style="grid-template-columns: repeat(auto-fill, minmax(14rem, 1fr))">
                    {planned
                        .into_iter()
                        .map(|r| {
                            let slug = r.slug.clone();
                            let class = format!(
                                "flex flex-col gap-1 rounded border-2 p-3 text-left hover:brightness-95 {}",
                                tone(&r),
                            );
                            view! {
                                <button
                                    type="button"
                                    class=class
                                    on:click=move |_| open.set(Some(slug.clone()))
                                >
                                    <span class="font-medium">{r.board.clone()}</span>
                                    <span class="text-xs">
                                        {r
                                            .duration
                                            .map(|d| format!("Duration {}", days_label(i64::from(d))))
                                            .unwrap_or_else(|| "No duration".to_string())}
                                    </span>
                                    <span class="text-xs">
                                        {r.remaining.map(left).unwrap_or_default()}
                                    </span>
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
                {(!unassigned.is_empty())
                    .then(|| {
                        view! {
                            <h2 class="mt-6 text-sm font-medium">"Roadmaps without a board"</h2>
                            <p class="text-xs text-slate-500">
                                "Made before roadmaps belonged to a board. Choose the board each plans."
                            </p>
                            <div class="mt-2 flex flex-col gap-2">
                                {unassigned
                                    .into_iter()
                                    .map(|r| {
                                        view! {
                                            <Unassigned roadmap=r free=free.clone() reload />
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        }
                    })}
            </div>
            {(!data.free_boards.is_empty())
                .then(|| {
                    view! {
                        <form
                            class="flex w-60 flex-col gap-2"
                            on:submit=move |ev| {
                                ev.prevent_default();
                                create();
                            }
                        >
                            <label class="flex flex-col gap-1 text-sm">
                                "Board"
                                <select
                                    class=SELECT
                                    prop:value=move || board.get()
                                    on:change=move |ev| board.set(event_target_value(&ev))
                                >
                                    {data
                                        .free_boards
                                        .iter()
                                        .map(|b| view! { <option value=b.clone()>{b.clone()}</option> })
                                        .collect_view()}
                                </select>
                            </label>
                            <button type="submit" class=PRIMARY disabled=move || busy.get()>
                                "Create Roadmap"
                            </button>
                            <Problem problem />
                        </form>
                    }
                })}
        </div>
    }
}

#[component]
fn Unassigned(
    roadmap: RoadmapCard,
    free: Vec<String>,
    reload: impl Fn() + Copy + 'static,
) -> impl IntoView {
    let ctx = Ctx::get();
    let problem = RwSignal::new(None::<String>);
    let board = RwSignal::new(free.first().cloned().unwrap_or_default());
    let slug = roadmap.slug.clone();
    let adopt = move || {
        let slug = slug.clone();
        spawn_local(async move {
            match api::adopt_roadmap(&slug, &board.get_untracked()).await {
                Ok(report) => {
                    ctx.report(report);
                    reload();
                }
                Err(e) => problem.set(Some(e)),
            }
        });
    };
    let can = !free.is_empty();
    view! {
        <div class="flex flex-wrap items-center gap-2 text-sm">
            <span class="font-medium">{roadmap.name.clone()}</span>
            <select
                class=SELECT
                disabled=!can
                prop:value=move || board.get()
                on:change=move |ev| board.set(event_target_value(&ev))
            >
                {free.iter().map(|b| view! { <option value=b.clone()>{b.clone()}</option> }).collect_view()}
            </select>
            <button type="button" class=SECONDARY disabled=!can on:click=move |_| adopt()>
                "Use this board"
            </button>
            <Problem problem />
        </div>
    }
}

#[component]
fn RoadmapView(
    roadmap: RoadmapCard,
    open: RwSignal<Option<String>>,
    reload: impl Fn() + Copy + 'static,
) -> impl IntoView {
    let ctx = Ctx::get();
    let busy = RwSignal::new(false);
    let problem = RwSignal::new(None::<String>);
    let saved = RwSignal::new(None::<String>);
    let goal = RwSignal::new(roadmap.goal.clone());
    let sprints = RwSignal::new(roadmap.sprints);
    let name = RwSignal::new(String::new());
    let description = RwSignal::new(String::new());
    let start = RwSignal::new(String::new());
    let due = RwSignal::new(String::new());
    let per_week = RwSignal::new(String::new());
    let duration = RwSignal::new(String::new());
    let slug = StoredValue::new(roadmap.slug.clone());

    let save = move || {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            let result = api::save_roadmap(
                &slug.get_value(),
                &goal.get_untracked(),
                sprints.get_untracked(),
            )
            .await;
            match result {
                Ok(report) => {
                    ctx.report(report);
                    problem.set(None);
                    saved.set(Some("Saved.".to_string()));
                    reload();
                }
                Err(e) => problem.set(Some(e)),
            }
            busy.set(false);
        });
    };

    let add = move || {
        if busy.get_untracked() {
            return;
        }
        let number = |text: String| text.trim().parse::<u32>().unwrap_or(0);
        busy.set(true);
        spawn_local(async move {
            // The goal and the sprint choice decide what the milestone makes,
            // so they are saved first.
            let saved_first = api::save_roadmap(
                &slug.get_value(),
                &goal.get_untracked(),
                sprints.get_untracked(),
            )
            .await;
            if let Err(e) = saved_first {
                problem.set(Some(e));
                busy.set(false);
                return;
            }
            let milestone = MilestoneCard {
                name: name.get_untracked(),
                description: description.get_untracked(),
                start: start.get_untracked(),
                due: due.get_untracked(),
                days_per_week: number(per_week.get_untracked()),
                duration: number(duration.get_untracked()),
                ..MilestoneCard::default()
            };
            match api::add_milestone(&slug.get_value(), &milestone).await {
                Ok(report) => {
                    ctx.report(report);
                    for field in [name, description, start, due, per_week, duration] {
                        field.set(String::new());
                    }
                    problem.set(None);
                    // The board gains the epic.
                    ctx.reload();
                    reload();
                }
                Err(e) => problem.set(Some(e)),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="flex items-center gap-3">
            <button type="button" class=SECONDARY on:click=move |_| open.set(None)>
                "← Roadmaps"
            </button>
            <h1 class="text-xl font-semibold">{roadmap.board.clone()}</h1>
            <span class="text-sm text-slate-500">
                {roadmap
                    .duration
                    .map(|d| format!("Duration {}", days_label(i64::from(d))))
                    .unwrap_or_default()}
                {roadmap.remaining.map(|r| format!(" · {}", left(r))).unwrap_or_default()}
            </span>
        </div>
        <section class="flex flex-col gap-3 rounded border border-slate-200 p-3 dark:border-slate-700">
            <label class="flex flex-col gap-1 text-sm">
                "Description and goal"
                <textarea
                    rows="4"
                    class=FIELD
                    prop:value=move || goal.get()
                    on:input=move |ev| {
                        goal.set(event_target_value(&ev));
                        saved.set(None);
                    }
                ></textarea>
            </label>
            <label class="flex items-center gap-2 text-sm">
                <input
                    type="checkbox"
                    prop:checked=move || sprints.get()
                    on:change=move |ev| {
                        sprints.set(event_target_checked(&ev));
                        saved.set(None);
                    }
                />
                "Use Sprints"
                <span class="text-xs text-slate-500">
                    "Milestones made from now on are sprints rather than epics."
                </span>
            </label>
            <div class="flex items-center gap-3">
                <button type="button" class=PRIMARY disabled=move || busy.get() on:click=move |_| save()>
                    "Save"
                </button>
                <span class="text-sm text-slate-500">{move || saved.get()}</span>
            </div>
        </section>
        <section class="flex flex-col gap-2">
            <h2 class="text-lg font-medium">"Milestones"</h2>
            {roadmap.milestones.is_empty().then(|| {
                view! { <p class="text-sm text-slate-500">"No milestones yet."</p> }
            })}
            <ul class="flex flex-col gap-2">
                {roadmap
                    .milestones
                    .iter()
                    .map(|m| {
                        let class = format!(
                            "rounded border-l-4 bg-slate-50 p-2 text-sm dark:bg-slate-800 {}",
                            if m.late {
                                "border-red-600"
                            } else if m.total.is_some() || m.remaining.is_some() {
                                "border-green-600"
                            } else {
                                "border-slate-300 dark:border-slate-600"
                            },
                        );
                        view! {
                            <li class=class>
                                <span class="font-medium">{m.name.clone()}</span>
                                <span class="text-slate-500">
                                    {format!(" · {}", m.kind)}
                                    {dated("starts", &m.start)}
                                    {dated("due by", &m.due)}
                                    {m
                                        .total
                                        .map(|d| format!(" · {}", days_label(i64::from(d))))
                                        .unwrap_or_default()}
                                    {m.remaining.map(|r| format!(" · {}", left(r))).unwrap_or_default()}
                                </span>
                                <p class="whitespace-pre-wrap">{m.description.clone()}</p>
                            </li>
                        }
                    })
                    .collect_view()}
            </ul>
        </section>
        <form
            class="flex flex-col gap-2 rounded border border-slate-200 p-3 dark:border-slate-700"
            on:submit=move |ev| {
                ev.prevent_default();
                add();
            }
        >
            <h3 class="text-sm font-medium">"New milestone"</h3>
            <label class="flex flex-col gap-1 text-sm">
                "Short name"
                <input
                    type="text"
                    required
                    class=FIELD
                    prop:value=move || name.get()
                    on:input=move |ev| name.set(event_target_value(&ev))
                />
            </label>
            <label class="flex flex-col gap-1 text-sm">
                "Description"
                <textarea
                    rows="3"
                    class=FIELD
                    prop:value=move || description.get()
                    on:input=move |ev| description.set(event_target_value(&ev))
                ></textarea>
            </label>
            <div class="flex flex-wrap gap-3">
                <label class="flex flex-col gap-1 text-sm">
                    "Start date"
                    <input
                        type="date"
                        class=FIELD
                        prop:value=move || start.get()
                        on:input=move |ev| start.set(event_target_value(&ev))
                    />
                </label>
                <label class="flex flex-col gap-1 text-sm">
                    "Due date"
                    <input
                        type="date"
                        class=FIELD
                        prop:value=move || due.get()
                        on:input=move |ev| due.set(event_target_value(&ev))
                    />
                </label>
                <label class="flex flex-col gap-1 text-sm">
                    "Days per week"
                    <input
                        type="number"
                        min="1"
                        max="7"
                        placeholder="5"
                        class=format!("{FIELD} w-24")
                        prop:value=move || per_week.get()
                        on:input=move |ev| per_week.set(event_target_value(&ev))
                    />
                </label>
                <label class="flex flex-col gap-1 text-sm">
                    "Duration (days)"
                    <input
                        type="number"
                        min="1"
                        class=format!("{FIELD} w-28")
                        prop:value=move || duration.get()
                        on:input=move |ev| duration.set(event_target_value(&ev))
                    />
                </label>
            </div>
            <p class="text-xs text-slate-500">
                {move || {
                    if sprints.get() {
                        "Makes a sprint of this name on the board, with this description, as long as the duration (else start to due)."
                    } else {
                        "Makes an epic of this name on the board."
                    }
                }}
            </p>
            <Problem problem />
            <div>
                <button type="submit" class=PRIMARY disabled=move || busy.get()>
                    "Add milestone"
                </button>
            </div>
        </form>
    }
}
