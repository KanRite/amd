//! Top-level state and layout.

use std::collections::BTreeSet;

use amd_ui_model::appearance::Appearance;
use amd_ui_model::epic::Place;
use amd_ui_model::ticket::SortOrder;
use amd_ui_model::{Assigned, Board, Column, SyncReport, TicketDocument, TicketSummary};
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;

use crate::admin::AdminPanel;
use crate::api;
use crate::board::BoardView;
use crate::dialog::{
    CloneRepoDialog, NewEpicDialog, NewSprintDialog, NewTicketDialog, OverLimitDialog,
};
use crate::editor::Editor;
use crate::preferences::PreferencesPanel;

/// Which overlay, if any, is open over the board.
#[derive(Clone, Debug, PartialEq)]
pub enum Overlay {
    Editor(TicketDocument),
    NewTicket,
    NewEpic,
    NewSprint,
    Admin,
    Preferences,
    /// A "(not cloned)" repository chosen in the filter: clone it? (108)
    CloneRepo(String),
    /// A ticket dropped on a column at its WIP limit: move it anyway?
    OverLimit {
        file: String,
        from: Place,
        to: Place,
        reason: String,
    },
}

/// State shared by every component, provided as context. All fields are
/// arena signals, so the struct is `Copy` and closures capture it freely.
#[derive(Clone, Copy)]
pub struct Ctx {
    pub board: RwSignal<Option<Board>>,
    pub error: RwSignal<Option<String>>,
    pub overlay: RwSignal<Option<Overlay>>,
    pub collapsed: RwSignal<BTreeSet<Column>>,
    /// The Epics column folds away on its own, separately from the backlog.
    pub epics_collapsed: RwSignal<bool>,
    /// The epic or sprint opened from the Backlogs column, as (repository,
    /// name), shown as a column of its own beside it (ticket 109). One at
    /// a time.
    pub open_epic: RwSignal<Option<(String, String)>>,
    /// The ticket being dragged: its file name and where it came from.
    pub dragging: RwSignal<Option<(String, Place)>>,
    /// How every column and epic is ordered (the Sort dropdown).
    pub sort: RwSignal<SortOrder>,
    /// The search box: only matching cards are shown while it has text.
    pub search: RwSignal<String>,
    /// Which repository's *board* new tickets, epics and WIP limits go to;
    /// `None` is the project's own. No longer set by the filter (ticket 101).
    pub repo: RwSignal<Option<String>>,
    /// The repository filter (ticket 101): only tickets *about* this
    /// repository (their `repository` field), or all (`None`).
    pub about: RwSignal<Option<String>>,
    /// The user filter: whose tickets to show.
    pub user: RwSignal<Assigned>,
    /// The user's colour scheme (Preferences → Appearance).
    pub appearance: RwSignal<Appearance>,
    /// The board's zoom, in percent (ticket 024).
    pub zoom: RwSignal<u32>,
    /// Whether the desktop is in dark mode, for "follow the system".
    pub system_dark: RwSignal<bool>,
    /// What git did with the last ticket change.
    pub sync: RwSignal<Option<SyncReport>>,
    /// Bumped whenever the board is refreshed because the files changed (or
    /// the user asked): an open editor checks its ticket then (ticket 091).
    pub disk_tick: RwSignal<u64>,
}

impl Ctx {
    pub fn get() -> Ctx {
        expect_context::<Ctx>()
    }

    /// Re-reads the board from disk.
    pub fn reload(self) {
        spawn_local(async move {
            match api::load_board().await {
                Ok(board) => self.board.set(Some(board)),
                Err(e) => self.error.set(Some(e)),
            }
        });
    }

    /// Opens a ticket in the editor.
    pub fn open_ticket(self, place: Place, file: String) {
        spawn_local(async move {
            match api::open_ticket(&place, &file).await {
                Ok(doc) => self.overlay.set(Some(Overlay::Editor(doc))),
                Err(e) => self.error.set(Some(e)),
            }
        });
    }

    /// The cards to show from a list: those matching the search, in the
    /// order chosen in the Sort dropdown.
    pub fn shown(self, mut list: Vec<TicketSummary>) -> Vec<TicketSummary> {
        list.retain(|t| self.in_view(t));
        let order = self.sort.get();
        list.sort_by(|a, b| order.compare(a, b));
        list
    }

    /// Whether a card passes the repository and user filters and the search.
    pub fn in_view(self, t: &TicketSummary) -> bool {
        let repo_ok = self
            .about
            .with(|r| r.as_ref().is_none_or(|r| *r == t.repository));
        repo_ok && self.user.with(|u| u.matches(t)) && self.search.with(|q| t.matches(q))
    }

    /// Whether any filter differs from its default: repository, user, sort
    /// or search.
    pub fn filters_changed(self) -> bool {
        self.about.with(Option::is_some)
            || self.user.with(|u| *u != Assigned::Anyone)
            || self.sort.get() != SortOrder::default()
            || self.searching()
    }

    /// Puts every filter back to its default: all repositories, all users,
    /// the default order, and no search.
    pub fn reset_filters(self) {
        self.about.set(None);
        self.user.set(Assigned::Anyone);
        self.sort.set(SortOrder::default());
        self.search.set(String::new());
    }

    /// Whether a search or the user filter is hiding some tickets, so epics
    /// left with nothing to show can step aside.
    pub fn narrowed(self) -> bool {
        self.searching() || self.user.with(|u| *u != Assigned::Anyone)
    }

    /// Whether the board spans more than one available repository.
    pub fn multi_repo(self) -> bool {
        self.board.with(|b| {
            b.as_ref()
                .is_some_and(|b| b.repos.iter().filter(|r| r.available).count() > 1)
        })
    }

    /// Whether a search is narrowing the board.
    pub fn searching(self) -> bool {
        self.search.with(|q| !q.trim().is_empty())
    }

    /// Re-reads the board because something outside the page may have
    /// changed it, and lets an open editor check its own ticket.
    pub fn refresh(self) {
        self.reload();
        self.disk_tick.update(|tick| *tick += 1);
    }

    /// Shows what happened when a change was committed and pushed.
    pub fn report(self, report: SyncReport) {
        self.sync.set(Some(report));
    }

    pub fn close_overlay(self) {
        self.overlay.set(None);
    }
}

#[component]
pub fn App() -> impl IntoView {
    let ctx = Ctx {
        board: RwSignal::new(None),
        error: RwSignal::new(None),
        overlay: RwSignal::new(None),
        collapsed: RwSignal::new(BTreeSet::new()),
        epics_collapsed: RwSignal::new(false),
        open_epic: RwSignal::new(None),
        dragging: RwSignal::new(None),
        sort: RwSignal::new(SortOrder::default()),
        search: RwSignal::new(String::new()),
        repo: RwSignal::new(None),
        about: RwSignal::new(None),
        user: RwSignal::new(Assigned::Anyone),
        sync: RwSignal::new(None),
        appearance: RwSignal::new(Appearance::default()),
        zoom: RwSignal::new(amd_ui_model::zoom::DEFAULT),
        system_dark: RwSignal::new(false),
        disk_tick: RwSignal::new(0),
    };
    provide_context(ctx);
    ctx.reload();
    use_colour_scheme(ctx);
    watch_disk(ctx);
    refresh_keys(ctx);
    zoom_keys(ctx);

    let tasks_dir = move || ctx.board.with(|b| b.as_ref().map(|b| b.tasks_dir.clone()));
    // The project's own name (ticket 087), until a board is open.
    let project = move || {
        ctx.board.with(|b| {
            b.as_ref()
                .map(|b| b.project.clone())
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| "agile-md".to_string())
        })
    };

    view! {
        <div class="flex h-screen flex-col bg-slate-100 text-slate-900 dark:bg-slate-900 dark:text-slate-100">
            <header class="flex items-center gap-4 border-b border-slate-200 bg-white px-4 py-2 dark:border-slate-700 dark:bg-slate-800">
                <ProjectSwitcher project=project />
                <span class="min-w-0 flex-1 truncate text-xs text-slate-500" title=tasks_dir>
                    {move || tasks_dir().unwrap_or_else(|| "No tasks directory open".into())}
                </span>
                <SyncStatus />
                <button
                    class="rounded border border-slate-300 px-2.5 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                    title="Refresh the board from disk (F5)"
                    aria-label="Refresh"
                    on:click=move |_| ctx.refresh()
                >
                    "↻"
                </button>
                <button
                    class="whitespace-nowrap rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50"
                    disabled=move || ctx.board.with(Option::is_none)
                    on:click=move |_| ctx.overlay.set(Some(Overlay::NewTicket))
                >
                    "New ticket"
                </button>
                <button
                    class="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                    title="Your own settings on this machine: colours, zoom, the board to open"
                    on:click=move |_| ctx.overlay.set(Some(Overlay::Preferences))
                >
                    "Preferences"
                </button>
                // Only for the project's admins (ticket 149): its admins list,
                // or admin access on GitHub, or open when amd can't tell.
                <Show when=move || {
                    ctx.board.with(|b| b.as_ref().is_none_or(|b| b.admin.allowed))
                }>
                    <button
                        class="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                        title="The project's settings, shared with the team"
                        on:click=move |_| ctx.overlay.set(Some(Overlay::Admin))
                    >
                        "Admin"
                    </button>
                </Show>
            </header>

            <FilterBar />

            <Show when=move || ctx.error.with(Option::is_some)>
                <div
                    role="alert"
                    class="flex items-center gap-3 bg-red-100 px-4 py-2 text-sm text-red-800 dark:bg-red-950 dark:text-red-200"
                >
                    <span class="flex-1">{move || ctx.error.get().unwrap_or_default()}</span>
                    <button class="underline" on:click=move |_| ctx.error.set(None)>
                        "Dismiss"
                    </button>
                </div>
            </Show>

            <Show when=move || ctx.sync.with(|s| s.as_ref().is_some_and(|s| s.warning.is_some()))>
                <div
                    role="alert"
                    class="flex items-center gap-3 bg-amber-100 px-4 py-2 text-sm text-amber-900 dark:bg-amber-950 dark:text-amber-100"
                >
                    <span class="flex-1">
                        {move || ctx.sync.with(|s| s.as_ref().and_then(|s| s.warning.clone()).unwrap_or_default())}
                    </span>
                    <button class="underline" on:click=move |_| ctx.sync.set(None)>
                        "Dismiss"
                    </button>
                </div>
            </Show>

            <main class="min-h-0 flex-1">
                <BoardView />
            </main>

            {move || match ctx.overlay.get() {
                Some(Overlay::Editor(doc)) => view! { <Editor doc /> }.into_any(),
                Some(Overlay::NewTicket) => view! { <NewTicketDialog /> }.into_any(),
                Some(Overlay::NewEpic) => view! { <NewEpicDialog /> }.into_any(),
                Some(Overlay::NewSprint) => view! { <NewSprintDialog /> }.into_any(),
                Some(Overlay::Admin) => view! { <AdminPanel /> }.into_any(),
                Some(Overlay::Preferences) => view! { <PreferencesPanel /> }.into_any(),
                Some(Overlay::CloneRepo(name)) => view! { <CloneRepoDialog name /> }.into_any(),
                Some(Overlay::OverLimit { file, from, to, reason }) => {
                    view! { <OverLimitDialog file from to reason /> }.into_any()
                }
                None => ().into_any(),
            }}
        </div>
    }
}

/// A centred modal over a dimmed board. Escape closes it; a click on the
/// backdrop deliberately does not, so a stray click cannot discard an edit.
/// Follows the files (ticket 091): asks for the board's fingerprint every
/// second and refreshes when it changes, so changes made anywhere else — a
/// terminal, an editor, a pull or a checkout — show up without a click. A
/// change seen mid-drag waits for the drop. The first answer, and the first
/// after switching project, is just the new baseline.
fn watch_disk(ctx: Ctx) {
    let last = StoredValue::new(None::<String>);
    let pending = StoredValue::new(false);
    let poll = move || {
        spawn_local(async move {
            if let Ok(now) = api::board_fingerprint().await {
                let before = last.get_value();
                if before.as_ref().is_some_and(|before| *before != now) {
                    pending.set_value(true);
                }
                last.set_value(Some(now));
            }
            if pending.get_value() && ctx.dragging.with_untracked(Option::is_none) {
                pending.set_value(false);
                ctx.refresh();
            }
        });
    };
    if let Err(e) = set_interval_with_handle(poll, std::time::Duration::from_secs(1)) {
        ctx.error
            .set(Some(format!("can't watch the board for changes: {e:?}")));
    }
    // Teammates' work (ticket 102): fetch and fast-forward every 30s; the
    // fingerprint poll above then sees the files change. Warnings (offline)
    // are left out of the way — every 30s would only nag.
    let sync = move || {
        spawn_local(async move {
            let _ = api::sync_board().await;
        });
    };
    let _ = set_interval_with_handle(sync, std::time::Duration::from_secs(30));
}

/// F5 and Ctrl+R refresh the board rather than reloading the whole page,
/// which would throw away the filters, the sort and anything being typed.
fn refresh_keys(ctx: Ctx) {
    let _keys = window_event_listener(leptos::ev::keydown, move |ev| {
        let reload_key = ev.key() == "F5"
            || ((ev.ctrl_key() || ev.meta_key()) && ev.key().eq_ignore_ascii_case("r"));
        if reload_key {
            ev.prevent_default();
            ctx.refresh();
        }
    });
}

/// Ctrl + and Ctrl - zoom the board a step, Ctrl 0 back to 100% (ticket 024),
/// remembered on this machine. The saved size is applied at start.
fn zoom_keys(ctx: Ctx) {
    spawn_local(async move {
        if let Ok(saved) = api::zoom().await {
            ctx.zoom.set(saved);
        }
    });
    let _keys = window_event_listener(leptos::ev::keydown, move |ev| {
        if !(ev.ctrl_key() || ev.meta_key()) {
            return;
        }
        use amd_ui_model::zoom;
        let now = ctx.zoom.get_untracked();
        let to = match ev.key().as_str() {
            "+" | "=" => zoom::step(now, true),
            "-" | "_" => zoom::step(now, false),
            "0" => zoom::DEFAULT,
            _ => return,
        };
        ev.prevent_default();
        zoom_to(ctx, to);
    });
}

/// Zooms the board to `to` percent and remembers it (ticket 024): the keys
/// and Preferences both come here.
pub fn zoom_to(ctx: Ctx, to: u32) {
    if to == ctx.zoom.get_untracked() {
        return;
    }
    // Set before the call returns, so a quick second press steps on from
    // here rather than from the old size.
    ctx.zoom.set(to);
    spawn_local(async move {
        if let Err(e) = api::set_zoom(to).await {
            ctx.error.set(Some(e));
        }
    });
}

#[component]
pub fn Modal(
    title: String,
    #[prop(default = "max-w-3xl")] width: &'static str,
    children: Children,
) -> impl IntoView {
    let ctx = Ctx::get();
    let label = title.clone();
    // Listen on the window: focus is not reliably inside the dialog (a newly
    // inserted `autofocus` field is not focused by WebKit), so a handler on
    // the dialog itself would miss the key.
    let escape = window_event_listener(leptos::ev::keydown, move |ev| {
        if ev.key() == "Escape" {
            ctx.close_overlay();
        }
    });
    on_cleanup(move || escape.remove());
    view! {
        <div class="fixed inset-0 z-10 flex items-center justify-center bg-black/40 p-6">
            <section
                role="dialog"
                aria-modal="true"
                aria-label=label
                class=format!(
                    "flex max-h-full w-full {width} flex-col gap-3 rounded-lg bg-white p-4 shadow-xl dark:bg-slate-800",
                )
            >
                <h2 class="text-base font-semibold">{title}</h2>
                {children()}
            </section>
        </div>
    }
}

/// The last commit the board made, in the header: "Pushed …" once it is on
/// origin, "Committed …" if it only got as far as the local repository.
#[component]
fn SyncStatus() -> impl IntoView {
    let ctx = Ctx::get();
    move || {
        ctx.sync.with(|s| {
            s.as_ref().filter(|s| s.committed).map(|s| {
                let (icon, tone, verb) = if s.pushed {
                    ("✓", "text-emerald-700 dark:text-emerald-300", "Pushed")
                } else {
                    ("!", "text-amber-700 dark:text-amber-300", "Committed")
                };
                view! {
                    <span role="status" class=format!("max-w-md truncate text-xs {tone}") title=s.message.clone()>
                        {format!("{icon} {verb} {}", s.message)}
                    </span>
                }
            })
        })
    }
}

/// Searches the tickets on the board (archived ones are not loaded). Escape
/// or the × clears it.
#[component]
fn SearchBox() -> impl IntoView {
    let ctx = Ctx::get();
    view! {
        <div class="relative flex items-center">
            <span aria-hidden="true" class="pointer-events-none absolute left-2.5 text-sm text-slate-400">
                "⌕"
            </span>
            <input
                type="search"
                placeholder="Search tickets"
                aria-label="Search tickets"
                class="w-56 rounded border border-slate-300 bg-white py-1.5 pr-7 pl-7 text-sm text-slate-900 placeholder:text-slate-400 focus:outline-2 focus:outline-accent-500 dark:border-slate-600 dark:bg-slate-900 dark:text-slate-100 [&::-webkit-search-cancel-button]:hidden"
                prop:value=move || ctx.search.get()
                on:input=move |ev| ctx.search.set(event_target_value(&ev))
                on:keydown=move |ev| {
                    if ev.key() == "Escape" {
                        ctx.search.set(String::new());
                    }
                }
            />
            <Show when=move || ctx.searching()>
                <button
                    type="button"
                    title="Clear the search"
                    aria-label="Clear the search"
                    class="absolute right-1.5 rounded px-1 text-sm text-slate-400 hover:text-slate-700 dark:hover:text-slate-200"
                    on:click=move |_| ctx.search.set(String::new())
                >
                    "×"
                </button>
            </Show>
        </div>
    }
}

/// The repository filter, at the far left: all repositories the project
/// spans, or one. Same indigo pill as Sort (and, like it, painted itself:
/// WebKit's native select box would hide white text).
#[component]
fn RepoFilter() -> impl IntoView {
    let ctx = Ctx::get();
    let repos = move || {
        ctx.board.with(|b| {
            b.as_ref()
                .map(|b| b.ticket_repos.clone())
                .unwrap_or_default()
        })
    };
    let shown = move || ctx.about.get().unwrap_or_else(|| "All".to_string());
    // A fixed width (tickets 096/100): a long repository name is cut short
    // here, in full on hover, and in full in the opened list, so it never
    // widens the filter or pushes the others onto another row.
    view! {
        <label
            class="group relative flex w-64 shrink-0 cursor-pointer items-center gap-1 whitespace-nowrap rounded bg-accent-600 py-1.5 pr-2 pl-3 text-sm font-medium text-white hover:bg-accent-500 focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-accent-500"
            title=move || format!("Repository: {} — show only the tickets about one repository", shown())
        >
            "Repository:"
            <select
                class="min-w-0 flex-1 cursor-pointer appearance-none truncate border-0 bg-accent-600 pr-5 font-medium text-white group-hover:bg-accent-500 focus:outline-none"
                prop:value=move || ctx.about.get().unwrap_or_default()
                on:change=move |ev| {
                    let value = event_target_value(&ev);
                    // One that isn't cloned here offers to clone it (ticket
                    // 108); the filter stays as it was until that's done.
                    let missing = repos().iter().any(|r| r.name == value && !r.available);
                    if missing {
                        let select: web_sys::HtmlSelectElement = event_target(&ev);
                        select.set_value(&ctx.about.get_untracked().unwrap_or_default());
                        ctx.overlay.set(Some(Overlay::CloneRepo(value)));
                    } else {
                        ctx.about.set((!value.is_empty()).then_some(value));
                    }
                }
            >
                <option class="bg-accent-600 text-white" value="">
                    "All"
                </option>
                {move || {
                    repos()
                        .into_iter()
                        .map(|r| {
                            let label = if r.available {
                                r.name.clone()
                            } else {
                                format!("{} (not cloned — clone…)", r.name)
                            };
                            view! {
                                <option class="bg-accent-600 text-white" value=r.name.clone()>
                                    {label}
                                </option>
                            }
                        })
                        .collect_view()
                }}
            </select>
            <span aria-hidden="true" class="pointer-events-none absolute right-2.5 text-xs text-white">
                "▾"
            </span>
        </label>
    }
}

/// The user filter, right of the repository filter: everyone's tickets, one
/// user's, or the unassigned ones. Users are whoever has tickets assigned.
#[component]
fn UserFilter() -> impl IntoView {
    let ctx = Ctx::get();
    let users = move || {
        ctx.board
            .with(|b| b.as_ref().map(Board::assignees).unwrap_or_default())
    };
    view! {
        <label
            class="group relative flex cursor-pointer items-center gap-1 whitespace-nowrap rounded bg-accent-600 py-1.5 pr-2 pl-3 text-sm font-medium text-white hover:bg-accent-500 focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-accent-500"
            title="Show the tickets assigned to one user, or everyone's"
        >
            "User:"
            <select
                class="cursor-pointer appearance-none border-0 bg-accent-600 pr-5 font-medium text-white group-hover:bg-accent-500 focus:outline-none"
                prop:value=move || ctx.user.with(Assigned::key)
                on:change=move |ev| ctx.user.set(Assigned::from_key(&event_target_value(&ev)))
            >
                <option class="bg-accent-600 text-white" value=Assigned::Anyone.key()>
                    "All"
                </option>
                {move || {
                    users()
                        .into_iter()
                        .map(|name| {
                            view! {
                                <option class="bg-accent-600 text-white" value=name.clone()>
                                    {name.clone()}
                                </option>
                            }
                        })
                        .collect_view()
                }}
                <option class="bg-accent-600 text-white" value=Assigned::Nobody.key()>
                    "Unassigned"
                </option>
            </select>
            <span aria-hidden="true" class="pointer-events-none absolute right-2.5 text-xs text-white">
                "▾"
            </span>
        </label>
    }
}

/// The row of filters across the top of the board: which repository and
/// whose work, a search, the order, and a reset back to the defaults.
#[component]
fn FilterBar() -> impl IntoView {
    let ctx = Ctx::get();
    view! {
        <nav
            aria-label="Filters"
            class="flex flex-wrap items-center gap-3 border-b border-slate-200 bg-slate-50 px-4 py-2 dark:border-slate-700 dark:bg-slate-800/60"
        >
            <Marked active=Signal::derive(move || ctx.repo.with(Option::is_some))>
                <RepoFilter />
            </Marked>
            <Marked active=Signal::derive(move || ctx.user.with(|u| *u != Assigned::Anyone))>
                <UserFilter />
            </Marked>
            <Marked active=Signal::derive(move || ctx.searching())>
                <SearchBox />
            </Marked>
            <Marked active=Signal::derive(move || ctx.sort.get() != SortOrder::default())>
            // WebKit draws a native (white) box for a <select> unless its
            // appearance is turned off, so the select is unstyled and
            // painted indigo itself, with its own arrow.
            <label
                class="group relative flex cursor-pointer items-center gap-1 whitespace-nowrap rounded bg-accent-600 py-1.5 pr-2 pl-3 text-sm font-medium text-white hover:bg-accent-500 focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-accent-500"
                title="Order of the tickets in every column"
            >
                "Sort:"
                <select
                    class="cursor-pointer appearance-none border-0 bg-accent-600 pr-5 font-medium text-white group-hover:bg-accent-500 focus:outline-none"
                    prop:value=move || ctx.sort.get().key()
                    on:change=move |ev| ctx.sort.set(SortOrder::from_key(&event_target_value(&ev)))
                >
                    // The open list is indigo too, with white text.
                    {SortOrder::ALL
                        .map(|o| {
                            view! {
                                <option class="bg-accent-600 text-white" value=o.key()>
                                    {o.label()}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
                <span aria-hidden="true" class="pointer-events-none absolute right-2.5 text-xs text-white">
                    "▾"
                </span>
            </label>
            </Marked>
            <button
                class="mt-2 whitespace-nowrap rounded border border-slate-300 bg-white px-3 py-1.5 text-sm hover:bg-slate-100 disabled:opacity-40 dark:border-slate-600 dark:bg-slate-900 dark:hover:bg-slate-700"
                title="Put Repository, User, Sort and the search back to their defaults"
                disabled=move || !ctx.filters_changed()
                on:click=move |_| ctx.reset_filters()
            >
                "Reset filters"
            </button>
        </nav>
    }
}

/// A filter in the filter row, with a coloured line above it while the
/// filter is active (differs from its default). The line's space is always
/// kept, so controls do not shift as filters change.
#[component]
fn Marked(active: Signal<bool>, children: Children) -> impl IntoView {
    view! {
        <div class="flex flex-col gap-1" data-active=move || active.get().to_string()>
            <span
                aria-hidden="true"
                class=move || {
                    format!(
                        "h-1 rounded-full transition-colors {}",
                        if active.get() { "bg-amber-400" } else { "bg-transparent" },
                    )
                }
            ></span>
            {children()}
            <span class="sr-only">{move || if active.get() { "Filter active" } else { "" }}</span>
        </div>
    }
}

/// Applies the user's colour scheme to the page: their saved choice is
/// loaded once, the desktop's light/dark setting is watched for "follow the
/// system", and `<html>` gets `class="dark"` and `data-accent` to match.
fn use_colour_scheme(ctx: Ctx) {
    spawn_local(async move {
        if let Ok(saved) = api::appearance().await {
            ctx.appearance.set(saved);
        }
    });
    if let Some(query) =
        web_sys::window().and_then(|w| w.match_media("(prefers-color-scheme: dark)").ok().flatten())
    {
        ctx.system_dark.set(query.matches());
        let watch = wasm_bindgen::closure::Closure::<dyn Fn()>::new({
            let query = query.clone();
            move || ctx.system_dark.set(query.matches())
        });
        let _ = query.add_event_listener_with_callback("change", watch.as_ref().unchecked_ref());
        // The listener lives as long as the page does.
        watch.forget();
    }
    Effect::new(move |_| {
        let Appearance { theme, accent } = ctx.appearance.get();
        let dark = theme.is_dark(ctx.system_dark.get());
        if let Some(root) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.document_element())
        {
            let _ = root.class_list().toggle_with_force("dark", dark);
            let _ = root.set_attribute("data-accent", accent.key());
        }
    });
}

/// The project's name, at the left of the header; a select when the folder
/// it lives in holds more than one project (ticket 093), switching to the
/// one picked.
#[component]
fn ProjectSwitcher(project: impl Fn() -> String + Send + Sync + Copy + 'static) -> impl IntoView {
    let ctx = Ctx::get();
    let choices = move || {
        ctx.board
            .with(|b| b.as_ref().map(|b| b.projects.clone()).unwrap_or_default())
    };
    let switch = move |name: String| {
        spawn_local(async move {
            match api::use_project(&name).await {
                Ok(board) => {
                    ctx.board.set(Some(board));
                    ctx.error.set(None);
                }
                Err(e) => ctx.error.set(Some(e)),
            }
        });
    };
    view! {
        <Show
            when=move || { choices().len() > 1 }
            fallback=move || {
                view! { <h1 class="shrink-0 whitespace-nowrap text-lg font-semibold">{project}</h1> }
            }
        >
            <label
                class="relative flex shrink-0 cursor-pointer items-center rounded border border-slate-300 hover:bg-slate-100 focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-accent-500 dark:border-slate-600 dark:hover:bg-slate-700"
                title="Switch to another project in this folder"
            >
                <select
                    aria-label="Project"
                    class="cursor-pointer appearance-none border-0 bg-transparent py-1 pr-7 pl-2 text-lg font-semibold focus:outline-none"
                    on:change=move |ev| switch(event_target_value(&ev))
                >
                    {move || {
                        choices()
                            .into_iter()
                            .map(|choice| {
                                view! {
                                    <option
                                        class="bg-white text-slate-900 dark:bg-slate-800 dark:text-slate-100"
                                        value=choice.name.clone()
                                        selected=choice.current
                                    >
                                        {choice.name.clone()}
                                    </option>
                                }
                            })
                            .collect_view()
                    }}
                </select>
                <span aria-hidden="true" class="pointer-events-none absolute right-2.5 text-xs text-slate-500">
                    "▾"
                </span>
            </label>
        </Show>
    }
}
