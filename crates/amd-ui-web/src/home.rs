//! The landing page (ticket 020): where amdui opens. Your work across every
//! project on this machine, highest priority first, a click away from its
//! ticket (or, with nothing to do, backlog tickets to pick, ticket 160);
//! and the projects, each a click away from its board. It's the home
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
                        <MyWork
                            work=h.work.clone()
                            to_pick=h.to_pick.clone()
                            me=h.me.clone()
                            include_unassigned=h.include_unassigned
                            home=home
                        />
                        <Projects projects=h.projects.clone() home=home />
                    }
                        .into_any()
                }
            }}
        </div>
    }
}

/// Opens `root`'s documents at one folder (ticket 168): `docs`, `adr` or
/// `ideas` of its own repository, or the shared repository.
fn open_documents(ctx: Ctx, root: String, shared: bool, folder: &str) {
    ctx.doc_scope.set(Some(amd_ui_model::DocScope {
        shared,
        folder: folder.to_string(),
    }));
    open(ctx, root, View::Documents, None);
}

/// Opens `root`'s project at `view` (its board or its documents), then,
/// if given, one of its tickets.
fn open(
    ctx: Ctx,
    root: String,
    view: View,
    ticket: Option<(String, amd_ui_model::Column, Option<String>, String)>,
) {
    spawn_local(async move {
        match api::set_tasks_dir(&root).await {
            Ok(board) => {
                ctx.board.set(Some(board));
                ctx.document.set(None);
                ctx.error.set(None);
                ctx.view.set(view);
                if let Some((repo, column, epic, file)) = ticket {
                    ctx.open_ticket(Place { repo, column, epic }, file);
                }
            }
            Err(e) => ctx.error.set(Some(e)),
        }
    });
}

/// How many tickets to pick are listed before "Show all" (ticket 171).
const PICK_SHOWN: usize = 5;

#[component]
fn MyWork(
    work: Vec<WorkItem>,
    to_pick: Vec<WorkItem>,
    me: String,
    include_unassigned: bool,
    home: RwSignal<Option<Result<Home, String>>>,
) -> impl IntoView {
    let ctx = Ctx::get();
    let set_unassigned = move |on: bool| {
        spawn_local(async move {
            match api::set_include_unassigned(on).await {
                Ok(()) => load_home(home),
                Err(e) => ctx.error.set(Some(e)),
            }
        });
    };
    let empty = work.is_empty();
    let picking = empty && !to_pick.is_empty();
    let note = if me.is_empty() {
        "git has no user.email, so amd can't tell which tickets are yours.".to_string()
    } else if picking {
        "Nothing in todo or doing is assigned to you. Work to pick:".to_string()
    } else {
        "Nothing in todo or doing is assigned to you.".to_string()
    };
    let rows = if empty { to_pick } else { work };
    // Work to pick can run long (every unassigned ticket at the top
    // priority), pushing Projects off the page: the first few, then Show
    // all (ticket 171). Your own work is always listed in full.
    let total = rows.len();
    let long = picking && total > PICK_SHOWN;
    let show_all = RwSignal::new(false);
    let rows = StoredValue::new(rows);
    view! {
        <section aria-labelledby="my-work" class="flex flex-col gap-2">
            <div class="flex items-baseline justify-between gap-3">
                <h2 id="my-work" class="text-lg font-semibold">"My work"</h2>
                <label
                    class="flex items-center gap-2 text-sm text-slate-600 dark:text-slate-300"
                    title="With nothing assigned to you: unassigned todo and doing tickets first, then unassigned backlog tickets by priority"
                >
                    <input
                        type="checkbox"
                        prop:checked=include_unassigned
                        on:change=move |ev| set_unassigned(event_target_checked(&ev))
                    />
                    "Include unassigned tickets"
                </label>
            </div>
            <Show when=move || empty>
                <p class="text-sm text-slate-500">{note.clone()}</p>
            </Show>
            <ul class="flex flex-col gap-1.5">
                {move || {
                    let limit = if long && !show_all.get() { PICK_SHOWN } else { total };
                    rows.get_value().into_iter().take(limit).map(work_row).collect_view()
                }}
            </ul>
            <Show when=move || long>
                <button
                    type="button"
                    class="self-start text-sm text-slate-500 hover:underline"
                    aria-expanded=move || show_all.get().to_string()
                    on:click=move |_| show_all.update(|s| *s = !*s)
                >
                    {move || {
                        if show_all.get() {
                            format!("Show the first {PICK_SHOWN}")
                        } else {
                            format!("Show all ({total})")
                        }
                    }}
                </button>
            </Show>
        </section>
    }
}

/// One ticket on the landing page: a click opens it on its board.
fn work_row(w: WorkItem) -> impl IntoView {
    let ctx = Ctx::get();
    let target = (w.repo.clone(), w.column, w.folder.clone(), w.file.clone());
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
                on:click=move |_| open(ctx, root.clone(), View::Board, Some(target.clone()))
            >
                <span class=format!("rounded px-1.5 text-xs font-medium {tone}")>{priority.clone()}</span>
                <span class="w-16 text-xs uppercase text-slate-500">{w.column.dir_name()}</span>
                <span class="font-mono text-xs text-slate-500">{w.id.clone()}</span>
                <span class="flex-1 font-medium">{w.title.clone()}</span>
                {(!w.points.is_empty())
                    .then(|| view! { <span class="font-mono text-xs text-slate-500">{format!("{} pts", w.points)}</span> })}
                <span class="text-xs text-slate-500">{w.project.clone()}</span>
            </button>
        </li>
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
        // Documents, ADRs and Ideas open folders of the project's own
        // documents repository; Shared, the one every project shares.
        // Greyed out when that repository isn't cloned here (ticket 168).
        let small = "rounded border border-slate-300 px-2 py-0.5 text-xs hover:bg-slate-100 disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent dark:border-slate-600 dark:hover:bg-slate-700";
        let doc_button = |label: &'static str, folder: &'static str, help: &'static str| {
            let root = p.root.clone();
            let available = p.documents;
            let title = if available {
                help.to_string()
            } else {
                format!("{}-documents isn't cloned on this machine", p.name)
            };
            view! {
                <button
                    type="button"
                    class=small
                    title=title
                    disabled=!available
                    on:click=move |_| open_documents(ctx, root.clone(), false, folder)
                >
                    {label}
                </button>
            }
        };
        let shared_button = {
            let root = p.root.clone();
            let available = p.shared_cloned;
            let title = match (&p.shared, available) {
                (Some(name), true) => format!("Read {name}, the documents every project shares"),
                (Some(name), false) => format!("{name} isn't cloned on this machine"),
                (None, _) => {
                    "This project names no shared documents (Admin → Shared documents)".to_string()
                }
            };
            view! {
                <button
                    type="button"
                    class=small
                    title=title
                    disabled=!available
                    on:click=move |_| open_documents(ctx, root.clone(), true, "")
                >
                    "Shared"
                </button>
            }
        };
        let hide = !p.hidden;
        view! {
            <li class="flex items-center gap-3 rounded-md border border-slate-200 bg-white px-3 py-2 text-sm dark:border-slate-700 dark:bg-slate-800">
                <button
                    type="button"
                    class="flex-1 text-left font-medium hover:underline"
                    title="Open this project's board"
                    on:click=move |_| open(ctx, root.clone(), View::Board, None)
                >
                    {p.name.clone()}
                    {show_dir.then(|| view! { <span class="ml-2 text-xs font-normal text-slate-500">{p.dir.clone()}</span> })}
                </button>
                {doc_button("Documents", "docs", "Read this project's documents (docs/)")}
                {doc_button("ADRs", "adr", "Read this project's decisions and proposals (adr/)")}
                {doc_button("Ideas", "ideas", "Read this project's ideas (ideas/)")}
                {shared_button}
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
