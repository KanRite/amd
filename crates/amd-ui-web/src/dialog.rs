//! Creating a ticket: ask for a title, render the template into the backlog,
//! then open the new ticket in the editor.

use amd_ui_model::epic::Place;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::app::{Ctx, Modal, Overlay};

#[component]
pub fn NewTicketDialog() -> impl IntoView {
    let ctx = Ctx::get();
    let title = RwSignal::new(String::new());
    let busy = RwSignal::new(false);

    let create = move || {
        let t = title.get_untracked();
        if t.trim().is_empty() || busy.get_untracked() {
            return;
        }
        busy.set(true);
        spawn_local(async move {
            // Onto the board being shown (the project's own under All), and
            // about the repository being filtered to, if any (ticket 101).
            let repo = ctx.repo.get_untracked();
            let about = ctx.about.get_untracked();
            match api::create_ticket(&t, repo.as_deref(), about.as_deref()).await {
                Ok(created) => {
                    ctx.report(created.sync);
                    ctx.reload();
                    ctx.overlay.set(Some(Overlay::Editor(created.doc)));
                }
                Err(e) => {
                    ctx.error.set(Some(e));
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
                        prop:value=move || title.get()
                        on:input=move |ev| title.set(event_target_value(&ev))
                    />
                </label>
                <p class="text-xs text-slate-500">
                    "The ticket is created in the backlog from the board's ticket template."
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
