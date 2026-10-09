//! The landing page (ticket 020): where amdui opens. Your work across every
//! project on this machine, highest priority first, a click away from its
//! ticket; and the projects, each a click away from its board. It's the home
//! that documents, ideas and collaboration will join.

use amd_ui_model::epic::Place;
use amd_ui_model::{Home, HomeProject, WorkItem};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::app::{Ctx, View};

/// Loads the landing page's data into `home`, reporting a failure on the
/// page rather than leaving it blank.
pub fn load_home(home: RwSignal<Option<Result<Home, String>>>) {
    spawn_local(async move { home.set(Some(api::home().await)) });
}

#[component]
pub fn HomeView(home: RwSignal<Option<Result<Home, String>>>) -> impl IntoView {
    view! {
        <div class="mx-auto flex h-full max-w-5xl flex-col gap-6 overflow-y-auto p-6">
            {move || match home.get() {
                None => view! { <p class="text-sm text-slate-500">"Loading your work…"</p> }.into_any(),
                Some(Err(e)) => {
                    view! {
                        <p role="alert" class="text-sm text-red-700 dark:text-red-300">
                            {format!("Couldn't read your projects: {e}")}
                        </p>
                    }
                        .into_any()
                }
                Some(Ok(h)) => {
                    view! {
                        <MyWork work=h.work.clone() me=h.me.clone() />
                        <Projects projects=h.projects.clone() home=home />
                    }
                        .into_any()
                }
            }}
        </div>
    }
}

/// Opens `root`'s project on the board, then, if given, one of its tickets.
fn open(ctx: Ctx, root: String, ticket: Option<(String, amd_ui_model::Column, String)>) {
    spawn_local(async move {
        match api::set_tasks_dir(&root).await {
            Ok(board) => {
                ctx.board.set(Some(board));
                ctx.error.set(None);
                ctx.view.set(View::Board);
                if let Some((repo, column, file)) = ticket {
                    ctx.open_ticket(
                        Place {
                            repo,
                            column,
                            epic: None,
                        },
                        file,
                    );
                }
            }
            Err(e) => ctx.error.set(Some(e)),
        }
    });
}

#[component]
fn MyWork(work: Vec<WorkItem>, me: String) -> impl IntoView {
    let ctx = Ctx::get();
    let empty = work.is_empty();
    let note = if me.is_empty() {
        "git has no user.email, so amd can't tell which tickets are yours.".to_string()
    } else {
        "Nothing in todo or doing is assigned to you.".to_string()
    };
    view! {
        <section aria-labelledby="my-work" class="flex flex-col gap-2">
            <h2 id="my-work" class="text-lg font-semibold">"My work"</h2>
            <Show when=move || empty>
                <p class="text-sm text-slate-500">{note.clone()}</p>
            </Show>
            <ul class="flex flex-col gap-1.5">
                {work
                    .into_iter()
                    .map(|w| {
                        let target = (w.repo.clone(), w.column, w.file.clone());
                        let root = w.project_root.clone();
                        let priority = w.priority.clone();
                        let tone = match priority.as_str() {
                            "high" => "bg-red-100 text-red-800 dark:bg-red-950 dark:text-red-200",
                            "medium" => "bg-amber-100 text-amber-800 dark:bg-amber-950 dark:text-amber-200",
                            _ => "bg-slate-200 text-slate-700 dark:bg-slate-700 dark:text-slate-200",
                        };
                        view! {
                            <li>
                                <button
                                    type="button"
                                    class="flex w-full items-baseline gap-3 rounded-md border border-slate-200 bg-white px-3 py-2 text-left text-sm hover:border-accent-400 dark:border-slate-700 dark:bg-slate-800"
                                    title="Open this ticket on its board"
                                    on:click=move |_| open(ctx, root.clone(), Some(target.clone()))
                                >
                                    <span class=format!("rounded px-1.5 text-xs font-medium {tone}")>{priority.clone()}</span>
                                    <span class="w-12 text-xs uppercase text-slate-500">{w.column.dir_name()}</span>
                                    <span class="font-mono text-xs text-slate-500">{w.id.clone()}</span>
                                    <span class="flex-1 font-medium">{w.title.clone()}</span>
                                    {(!w.points.is_empty())
                                        .then(|| view! { <span class="font-mono text-xs text-slate-500">{format!("{} pts", w.points)}</span> })}
                                    <span class="text-xs text-slate-500">{w.project.clone()}</span>
                                </button>
                            </li>
                        }
                    })
                    .collect_view()}
            </ul>
        </section>
    }
}

#[component]
fn Projects(
    projects: Vec<HomeProject>,
    home: RwSignal<Option<Result<Home, String>>>,
) -> impl IntoView {
    let ctx = Ctx::get();
    // The same name in two folders: say which folder, so they can be told
    // apart (ticket 095).
    let twice = |name: &str| projects.iter().filter(|p| p.name == name).count() > 1;
    let rows: Vec<(HomeProject, bool)> = projects
        .iter()
        .map(|p| (p.clone(), twice(&p.name)))
        .collect();
    let (shown, hidden): (Vec<_>, Vec<_>) = rows.into_iter().partition(|(p, _)| !p.hidden);
    let show_hidden = RwSignal::new(false);
    let hidden_count = hidden.len();
    let toggle = move |root: String, hide: bool| {
        spawn_local(async move {
            match api::set_project_hidden(&root, hide).await {
                Ok(()) => crate::home::load_home(home),
                Err(e) => ctx.error.set(Some(e)),
            }
        });
    };
    let row = move |(p, show_dir): (HomeProject, bool)| {
        let root = p.root.clone();
        let flip = p.root.clone();
        let hide = !p.hidden;
        view! {
            <li class="flex items-center gap-3 rounded-md border border-slate-200 bg-white px-3 py-2 text-sm dark:border-slate-700 dark:bg-slate-800">
                <button
                    type="button"
                    class="flex-1 text-left font-medium hover:underline"
                    title="Open this project's board"
                    on:click=move |_| open(ctx, root.clone(), None)
                >
                    {p.name.clone()}
                    {show_dir.then(|| view! { <span class="ml-2 text-xs font-normal text-slate-500">{p.dir.clone()}</span> })}
                </button>
                <button
                    type="button"
                    class="rounded border border-slate-300 px-2 py-0.5 text-xs hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                    title=if hide { "Leave this project out of My work (on this machine only)" } else { "Put this project back in My work" }
                    on:click=move |_| toggle(flip.clone(), hide)
                >
                    {if hide { "Hide" } else { "Show" }}
                </button>
            </li>
        }
    };
    view! {
        <section aria-labelledby="projects" class="flex flex-col gap-2">
            <h2 id="projects" class="text-lg font-semibold">"Projects"</h2>
            <ul class="flex flex-col gap-1.5">{shown.into_iter().map(row).collect_view()}</ul>
            <Show when=move || { hidden_count > 0 }>
                <button
                    type="button"
                    class="self-start text-sm text-slate-500 hover:underline"
                    aria-expanded=move || show_hidden.get().to_string()
                    on:click=move |_| show_hidden.update(|s| *s = !*s)
                >
                    {move || format!("{} Hidden ({hidden_count})", if show_hidden.get() { "▾" } else { "▸" })}
                </button>
            </Show>
            <Show when=move || show_hidden.get()>
                <ul class="flex flex-col gap-1.5 opacity-80">{hidden.clone().into_iter().map(row).collect_view()}</ul>
            </Show>
        </section>
    }
}
