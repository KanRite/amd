//! The Roadmaps view (ticket 206): a roadmap has a name and an estimated due
//! date; its milestones have a name, a description and a due-by date. Adding
//! a milestone makes an epic of the same name on the board, or, if asked, a
//! sprint that takes the description and the dates.

use amd_ui_model::{MilestoneCard, RoadmapCard};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::app::Ctx;

const FIELD: &str = "rounded border border-slate-300 bg-slate-50 px-2 py-1.5 text-sm dark:border-slate-600 dark:bg-slate-900";
const PRIMARY: &str = "rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50";

fn dated(label: &str, date: &str) -> String {
    if date.is_empty() {
        String::new()
    } else {
        format!(" · {label} {date}")
    }
}

#[component]
pub fn RoadmapsView() -> impl IntoView {
    let ctx = Ctx::get();
    let list = RwSignal::new(None::<Result<Vec<RoadmapCard>, String>>);
    let busy = RwSignal::new(false);
    let problem = RwSignal::new(None::<String>);

    let reload = move || spawn_local(async move { list.set(Some(api::list_roadmaps().await)) });
    reload();

    // New roadmap.
    let name = RwSignal::new(String::new());
    let due = RwSignal::new(String::new());
    let create = move || {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            match api::create_roadmap(&name.get_untracked(), &due.get_untracked()).await {
                Ok(report) => {
                    ctx.report(report);
                    name.set(String::new());
                    due.set(String::new());
                    problem.set(None);
                    reload();
                }
                Err(e) => problem.set(Some(e)),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="mx-auto flex h-full max-w-3xl flex-col gap-4 overflow-y-auto p-4">
            <h1 class="text-xl font-semibold">"Roadmaps"</h1>
            <form
                class="flex flex-wrap items-end gap-3 rounded border border-slate-200 p-3 dark:border-slate-700"
                on:submit=move |ev| {
                    ev.prevent_default();
                    create();
                }
            >
                <label class="flex flex-col gap-1 text-sm">
                    "Short name"
                    <input
                        type="text"
                        class=FIELD
                        prop:value=move || name.get()
                        on:input=move |ev| name.set(event_target_value(&ev))
                    />
                </label>
                <label class="flex flex-col gap-1 text-sm">
                    "Estimated due date"
                    <input
                        type="date"
                        class=FIELD
                        prop:value=move || due.get()
                        on:input=move |ev| due.set(event_target_value(&ev))
                    />
                </label>
                <button type="submit" class=PRIMARY disabled=move || busy.get()>
                    "Create roadmap"
                </button>
            </form>
            <Problem problem />
            {move || match list.get() {
                None => view! { <p class="text-sm text-slate-500">"Loading…"</p> }.into_any(),
                Some(Err(e)) => {
                    view! { <p role="alert" class="text-sm text-red-700 dark:text-red-300">{e}</p> }
                        .into_any()
                }
                Some(Ok(roadmaps)) if roadmaps.is_empty() => {
                    view! { <p class="text-sm text-slate-500">"No roadmaps yet."</p> }.into_any()
                }
                Some(Ok(roadmaps)) => {
                    roadmaps
                        .into_iter()
                        .map(|r| view! { <Roadmap roadmap=r reload /> })
                        .collect_view()
                        .into_any()
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

#[component]
fn Roadmap(roadmap: RoadmapCard, reload: impl Fn() + Copy + 'static) -> impl IntoView {
    let ctx = Ctx::get();
    let busy = RwSignal::new(false);
    let problem = RwSignal::new(None::<String>);
    let name = RwSignal::new(String::new());
    let description = RwSignal::new(String::new());
    let start = RwSignal::new(String::new());
    let due = RwSignal::new(String::new());
    let sprint = RwSignal::new(false);
    let slug = roadmap.slug.clone();

    let add = move || {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        let slug = slug.clone();
        spawn_local(async move {
            let milestone = MilestoneCard {
                name: name.get_untracked(),
                description: description.get_untracked(),
                kind: if sprint.get_untracked() {
                    "sprint"
                } else {
                    "epic"
                }
                .to_string(),
                start: start.get_untracked(),
                due: due.get_untracked(),
            };
            match api::add_milestone(&slug, &milestone).await {
                Ok(report) => {
                    ctx.report(report);
                    for field in [name, description, start, due] {
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
        <section class="flex flex-col gap-3 rounded border border-slate-200 p-3 dark:border-slate-700">
            <h2 class="text-lg font-medium">
                {roadmap.name.clone()}
                <span class="text-sm font-normal text-slate-500">
                    {dated("due", &roadmap.due)}
                </span>
            </h2>
            <ul class="flex flex-col gap-2">
                {roadmap
                    .milestones
                    .iter()
                    .map(|m| {
                        view! {
                            <li class="rounded bg-slate-50 p-2 text-sm dark:bg-slate-800">
                                <span class="font-medium">{m.name.clone()}</span>
                                <span class="text-slate-500">
                                    {format!(" · {}", m.kind)}
                                    {dated("starts", &m.start)}
                                    {dated("due by", &m.due)}
                                </span>
                                <p class="whitespace-pre-wrap">{m.description.clone()}</p>
                            </li>
                        }
                    })
                    .collect_view()}
            </ul>
            <form
                class="flex flex-col gap-2 border-t border-slate-200 pt-3 dark:border-slate-700"
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
                        "Due by"
                        <input
                            type="date"
                            class=FIELD
                            prop:value=move || due.get()
                            on:input=move |ev| due.set(event_target_value(&ev))
                        />
                    </label>
                    <label class="flex items-end gap-2 pb-1.5 text-sm">
                        <input
                            type="checkbox"
                            prop:checked=move || sprint.get()
                            on:change=move |ev| sprint.set(event_target_checked(&ev))
                        />
                        "Make it a sprint"
                    </label>
                    <Show when=move || sprint.get()>
                        <label class="flex flex-col gap-1 text-sm">
                            "Starts"
                            <input
                                type="date"
                                class=FIELD
                                prop:value=move || start.get()
                                on:input=move |ev| start.set(event_target_value(&ev))
                            />
                        </label>
                    </Show>
                </div>
                <p class="text-xs text-slate-500">
                    {move || {
                        if sprint.get() {
                            "Makes a sprint of this name on the board, with this description, as long as from its start to its due date."
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
        </section>
    }
}
