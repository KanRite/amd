//! Creating a ticket: ask for a title, render the template into the backlog,
//! then open the new ticket in the editor.

use amd_ui_model::epic::Place;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::app::{Ctx, LinkBack, LinkKind, Modal, Overlay};

#[component]
pub fn NewTicketDialog(link: Option<LinkBack>) -> impl IntoView {
    let ctx = Ctx::get();
    let title = RwSignal::new(String::new());
    // From Save and New (ticket 227): how it ties back, "requires" unless
    // changed.
    let kind = RwSignal::new(LinkKind::Requires);
    let link = StoredValue::new(link);
    let draft = RwSignal::new(false);
    let busy = RwSignal::new(false);
    // Why the create failed, shown in the dialog (ticket 021): the status
    // line is behind it, where a failure looked like a button doing nothing.
    let problem = RwSignal::new(None::<String>);

    let create = move || {
        let t = title.get_untracked();
        let as_draft = draft.get_untracked();
        if t.trim().is_empty() || busy.get_untracked() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            // Onto the board being shown (the project's own under All), and
            // about the repository being filtered to, if any (ticket 101).
            let back = link.get_value();
            // Linked tickets share a board, so it goes on the other's.
            let repo = match &back {
                Some(b) if !b.repo.is_empty() => Some(b.repo.clone()),
                _ => ctx.repo.get_untracked(),
            };
            let about = ctx.about.get_untracked();
            let tie = back
                .as_ref()
                .map(|b| (b.id.clone(), kind.get_untracked()))
                .filter(|(_, k)| *k != LinkKind::None);
            match api::create_ticket(
                &t,
                repo.as_deref(),
                about.as_deref(),
                as_draft,
                tie.as_ref().map(|(id, k)| (id.as_str(), *k)),
            )
            .await
            {
                Ok(created) => {
                    ctx.report(created.sync);
                    ctx.reload();
                    ctx.overlay.set(Some(Overlay::Editor(created.doc)));
                }
                Err(e) => {
                    problem.set(Some(e));
                    busy.set(false);
                }
            }
        });
    };

    view! {
        <Modal title="New ticket".to_string() width="max-w-lg">
            <form
                class="flex flex-col gap-3"
                on:submit=move |ev| {
                    ev.prevent_default();
                    create();
                }
            >
                <label class="flex flex-col gap-1 text-sm">
                    "Title"
                    <input
                        type="text"
                        autofocus
                        class="rounded border border-slate-300 bg-slate-50 px-2 py-1.5 dark:border-slate-600 dark:bg-slate-900"
                        aria-invalid=move || problem.with(Option::is_some).to_string()
                        prop:value=move || title.get()
                        on:input=move |ev| {
                            title.set(event_target_value(&ev));
                            problem.set(None);
                        }
                    />
                </label>
                {link
                    .get_value()
                    .map(|b| {
                        let name = format!("{} {}", b.id, b.title);
                        view! {
                            <label class="flex flex-col gap-1 text-sm">
                                "Link to"
                                <select
                                    class="appearance-none rounded border border-slate-300 bg-slate-50 px-2 py-1.5 dark:border-slate-600 dark:bg-slate-900"
                                    aria-label="How the new ticket links back"
                                    on:change=move |ev| {
                                        kind.set(match event_target_value(&ev).as_str() {
                                            "related" => LinkKind::Related,
                                            "none" => LinkKind::None,
                                            _ => LinkKind::Requires,
                                        })
                                    }
                                >
                                    <option value="requires" selected>{format!("{name} requires this")}</option>
                                    <option value="related">{format!("Related to {name}")}</option>
                                    <option value="none">"No link"</option>
                                </select>
                            </label>
                        }
                    })}
                <label class="flex items-center gap-2 text-sm">
                    <input
                        type="checkbox"
                        prop:checked=move || draft.get()
                        on:change=move |ev| draft.set(event_target_checked(&ev))
                    />
                    "Save as draft"
                </label>
                <p class="text-xs text-slate-500">
                    "The ticket is created in the backlog from the board's ticket template. A draft stays in the backlog until it's marked ready."
                </p>
                <ProblemNote problem />
                <div class="flex justify-end gap-2">
                    <button
                        type="button"
                        class="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                        on:click=move |_| ctx.close_overlay()
                    >
                        "Cancel"
                    </button>
                    <button
                        type="submit"
                        class="rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50"
                        disabled=move || busy.get() || title.with(|t| t.trim().is_empty())
                    >
                        "Create"
                    </button>
                </div>
            </form>
        </Modal>
    }
}

/// Creating an epic: a name, checked by the backend (required, unique, and
/// safe to use as the epic's folder name), with the reason shown inline.
#[component]
pub fn NewEpicDialog() -> impl IntoView {
    let ctx = Ctx::get();
    let name = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let problem = RwSignal::new(None::<String>);

    let create = move || {
        let n = name.get_untracked();
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            let repo = ctx.repo.get_untracked();
            match api::create_epic(&n, repo.as_deref()).await {
                Ok(report) => {
                    ctx.report(report);
                    ctx.close_overlay();
                    ctx.reload();
                }
                Err(e) => problem.set(Some(e)),
            }
            busy.set(false);
        });
    };

    view! {
        <Modal title="New epic".to_string() width="max-w-lg">
            <form
                class="flex flex-col gap-3"
                on:submit=move |ev| {
                    ev.prevent_default();
                    create();
                }
            >
                <label class="flex flex-col gap-1 text-sm">
                    "Name"
                    <input
                        type="text"
                        autofocus
                        aria-invalid=move || problem.with(Option::is_some).to_string()
                        class="rounded border border-slate-300 bg-slate-50 px-2 py-1.5 dark:border-slate-600 dark:bg-slate-900"
                        prop:value=move || name.get()
                        on:input=move |ev| {
                            name.set(event_target_value(&ev));
                            problem.set(None);
                        }
                    />
                </label>
                {move || {
                    problem
                        .get()
                        .map(|p| {
                            view! {
                                <p role="alert" class="text-sm font-medium text-red-700 dark:text-red-300">
                                    {p}
                                </p>
                            }
                        })
                }}
                <p class="text-xs text-slate-500">
                    "An epic groups backlog tickets. Drag tickets from the backlog into it, and back out again."
                </p>
                <div class="flex justify-end gap-2">
                    <button
                        type="button"
                        class="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                        on:click=move |_| ctx.close_overlay()
                    >
                        "Cancel"
                    </button>
                    <button
                        type="submit"
                        class="rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50"
                        disabled=move || busy.get()
                    >
                        "Create epic"
                    </button>
                </div>
            </form>
        </Modal>
    }
}

/// Creating a sprint: a name and a length in days (14 unless changed), as
/// `amd group sprint` takes them; checked by the backend, reasons inline.
#[component]
pub fn NewSprintDialog() -> impl IntoView {
    let ctx = Ctx::get();
    let name = RwSignal::new(String::new());
    let days = RwSignal::new(amd_ui_model::epic::DEFAULT_SPRINT_DAYS.to_string());
    let busy = RwSignal::new(false);
    let problem = RwSignal::new(None::<String>);

    let create = move || {
        if busy.get_untracked() {
            return;
        }
        let Ok(d) = days.get_untracked().trim().parse::<u32>() else {
            problem.set(Some(
                "Give the sprint's length as a whole number of days.".into(),
            ));
            return;
        };
        let n = name.get_untracked();
        busy.set(true);
        spawn_local(async move {
            let repo = ctx.repo.get_untracked();
            match api::create_sprint(&n, d, repo.as_deref()).await {
                Ok(report) => {
                    ctx.report(report);
                    ctx.close_overlay();
                    ctx.reload();
                }
                Err(e) => problem.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let input = "rounded border border-slate-300 bg-slate-50 px-2 py-1.5 dark:border-slate-600 dark:bg-slate-900";

    view! {
        <Modal title="New sprint".to_string() width="max-w-lg">
            <form
                class="flex flex-col gap-3"
                on:submit=move |ev| {
                    ev.prevent_default();
                    create();
                }
            >
                <div class="grid grid-cols-[1fr_7rem] gap-3">
                    <label class="flex flex-col gap-1 text-sm">
                        "Name"
                        <input
                            type="text"
                            autofocus
                            class=input
                            prop:value=move || name.get()
                            on:input=move |ev| {
                                name.set(event_target_value(&ev));
                                problem.set(None);
                            }
                        />
                    </label>
                    <label class="flex flex-col gap-1 text-sm">
                        "Days"
                        <input
                            type="number"
                            min="1"
                            max="365"
                            class=input
                            prop:value=move || days.get()
                            on:input=move |ev| {
                                days.set(event_target_value(&ev));
                                problem.set(None);
                            }
                        />
                    </label>
                </div>
                {move || {
                    problem
                        .get()
                        .map(|p| {
                            view! {
                                <p role="alert" class="text-sm font-medium text-red-700 dark:text-red-300">
                                    {p}
                                </p>
                            }
                        })
                }}
                <p class="text-xs text-slate-500">
                    "A sprint holds sized tickets (with points). Start it when the work is agreed: from then on it takes no more tickets and gives none back, and its tickets move on to To do."
                </p>
                <div class="flex justify-end gap-2">
                    <button
                        type="button"
                        class="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                        on:click=move |_| ctx.close_overlay()
                    >
                        "Cancel"
                    </button>
                    <button
                        type="submit"
                        class="rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50"
                        disabled=move || busy.get()
                    >
                        "Create sprint"
                    </button>
                </div>
            </form>
        </Modal>
    }
}

/// A ticket dropped on a column at its WIP limit: the reason, and the
/// choice to move it anyway (the deliberate override) or leave it.
#[component]
pub fn OverLimitDialog(file: String, from: Place, to: Place, reason: String) -> impl IntoView {
    let ctx = Ctx::get();
    let target = StoredValue::new((file, from, to));
    let move_anyway = move |_| {
        let (file, from, to) = target.get_value();
        ctx.close_overlay();
        crate::board::move_ticket(ctx, file, from, to, true);
    };
    view! {
        <Modal title="Column is full".to_string() width="max-w-lg">
            <p role="alert" class="text-sm">{reason}</p>
            <div class="flex justify-end gap-2">
                <button
                    type="button"
                    class="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                    on:click=move |_| ctx.close_overlay()
                >
                    "Leave it"
                </button>
                <button
                    type="button"
                    class="rounded bg-amber-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-amber-500"
                    on:click=move_anyway
                >
                    "Move it anyway"
                </button>
            </div>
        </Modal>
    }
}

/// A repository the project declares but this machine hasn't got, chosen in
/// the Repository filter (ticket 108): clone it beside the tickets
/// repository, as `amd clone <name>` does, then filter to it.
#[component]
pub fn CloneRepoDialog(name: String) -> impl IntoView {
    let ctx = Ctx::get();
    let busy = RwSignal::new(false);
    // A failed clone says why here and stays open (ticket 021).
    let problem = RwSignal::new(None::<String>);
    let target = StoredValue::new(name.clone());
    let clone = move |_| {
        if busy.get_untracked() {
            return;
        }
        busy.set(true);
        let name = target.get_value();
        spawn_local(async move {
            match api::clone_repository(&name).await {
                Ok(warnings) => {
                    if !warnings.is_empty() {
                        ctx.report(amd_ui_model::SyncReport {
                            message: format!("clone {name}"),
                            committed: false,
                            pushed: false,
                            warning: Some(warnings.join("; ")),
                        });
                    }
                    ctx.about.set(Some(name));
                    ctx.close_overlay();
                    ctx.reload();
                }
                Err(e) => {
                    problem.set(Some(e));
                    busy.set(false);
                }
            }
        });
    };
    view! {
        <Modal title=format!("Clone {name}?") width="max-w-lg">
            <p class="text-sm">
                {format!(
                    "{name} is part of this project but isn't cloned on this machine. Clone it beside the \
                     tickets repository, from the same place the tickets repository came from?"
                )}
            </p>
            <ProblemNote problem />
            <div class="flex justify-end gap-2">
                <button
                    type="button"
                    class="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                    on:click=move |_| ctx.close_overlay()
                >
                    "Cancel"
                </button>
                <button
                    type="button"
                    class="rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50"
                    disabled=move || busy.get()
                    on:click=clone
                >
                    {move || if busy.get() { "Cloning…" } else { "Clone" }}
                </button>
            </div>
        </Modal>
    }
}

/// Why a dialog's action failed, shown inside the dialog next to its
/// buttons (ticket 021), rather than on the status line behind it.
#[component]
fn ProblemNote(problem: RwSignal<Option<String>>) -> impl IntoView {
    move || {
        problem.get().map(|p| {
            view! {
                <p role="alert" class="text-sm font-medium text-red-700 dark:text-red-300">
                    {p}
                </p>
            }
        })
    }
}
