//! Swim lanes (ticket 218): other projects' boards under the one on
//! screen, each a row whose Backlog, To do, Doing and Done line up under
//! the board's own. View only: nothing drags, and a card opens read-only
//! with a way to its own board. Which projects is a Preference.

use amd_ui_model::{Column, Lane, LaneTicket, TicketSummary};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::app::{Ctx, Modal, Overlay, View};
use crate::board::{LockBadge, priority_tone};

const FRAME: &str = "flex min-h-0 flex-col rounded-lg bg-slate-200/50 dark:bg-slate-800/50";

/// One project's lane: its header where the board has Backlogs, a blank
/// where an epic column is open, then its four columns.
#[component]
pub fn LaneRow(lane: Lane) -> impl IntoView {
    let ctx = Ctx::get();
    let root = StoredValue::new(lane.root.clone());
    let folded = move || ctx.lanes_folded.with(|f| f.contains(&root.get_value()));
    let toggle = move |_| {
        let root = root.get_value();
        ctx.lanes_folded.update(|f| {
            if !f.remove(&root) {
                f.insert(root);
            }
        });
    };
    let total: usize = lane.columns.iter().map(|c| c.tickets.len()).sum();
    let columns = StoredValue::new(lane.columns.clone());
    let open_board = move |_| {
        crate::home::open(ctx, root.get_value(), View::Board, None);
    };
    let small_button = "whitespace-nowrap rounded px-1.5 text-xs text-slate-600 hover:bg-slate-300 dark:text-slate-300 dark:hover:bg-slate-700";
    view! {
        <div class="flex shrink-0 gap-3" role="region" aria-label=format!("Swim lane: {}", lane.name)>
            // Where the board has its Backlogs column (folded or not).
            <section class=move || {
                if ctx.epics_collapsed.get() {
                    format!("{FRAME} w-10 shrink-0 items-center overflow-hidden")
                } else {
                    format!("{FRAME} min-w-48 flex-1 basis-0")
                }
            }>
                <header class="flex flex-wrap items-center gap-1 px-3 py-2">
                    <button
                        type="button"
                        class="text-xs text-slate-500"
                        title=move || if folded() { "Show this lane's cards" } else { "Fold this lane" }
                        aria-expanded=move || (!folded()).to_string()
                        on:click=toggle
                    >
                        {move || if folded() { "▸" } else { "▾" }}
                    </button>
                    <h2 class="truncate text-sm font-semibold" title=format!("{} (view only)", lane.name)>
                        {lane.name.clone()}
                    </h2>
                    <span class="rounded-full bg-slate-300 px-2 text-xs dark:bg-slate-700">{total}</span>
                    {lane.archived.then(|| view! { <span class="text-xs italic text-slate-500">"archived"</span> })}
                    <span class="flex-1"></span>
                    <button class=small_button title="Go to this project's board to change its tickets" on:click=open_board>
                        "Open board"
                    </button>
                </header>
                <p class="px-3 pb-2 text-xs text-slate-500">"View only"</p>
                {lane.error.clone().map(|e| view! { <p class="px-3 pb-2 text-xs text-red-700 dark:text-red-300">{e}</p> })}
            </section>
            // An epic column open on the board: keep the columns lined up.
            {move || ctx.open_epic.get().map(|_| view! { <div class="min-w-48 flex-1 basis-0"></div> })}
            {Column::ALL
                .map(|column| {
                    let collapsed = move || ctx.collapsed.with(|c| c.contains(&column));
                    let tickets = move || {
                        let mut list: Vec<TicketSummary> = columns
                            .with_value(|cs| {
                                cs.iter()
                                    .find(|c| c.column == column)
                                    .map(|c| c.tickets.clone())
                                    .unwrap_or_default()
                            });
                        list.retain(|t| ctx.in_lane(t));
                        let order = ctx.sort.get();
                        list.sort_by(|a, b| order.compare(a, b));
                        list
                    };
                    view! {
                        <section
                            aria-label=format!("{} ({})", column.title(), lane.name)
                            class=move || {
                                if collapsed() {
                                    format!("{FRAME} w-10 shrink-0")
                                } else {
                                    format!("{FRAME} min-w-48 flex-1 basis-0")
                                }
                            }
                        >
                            <Show when=move || !collapsed() && !folded()>
                                <ul class="flex max-h-72 flex-col gap-2 overflow-y-auto p-2">
                                    {move || {
                                        tickets()
                                            .into_iter()
                                            .map(|t| view! { <LaneCard root=root.get_value() ticket=t /> })
                                            .collect_view()
                                    }}
                                </ul>
                            </Show>
                            <Show when=move || !collapsed() && folded()>
                                <p class="px-3 py-2 text-xs text-slate-500">
                                    {move || format!("{} {}", tickets().len(), column.title().to_lowercase())}
                                </p>
                            </Show>
                        </section>
                    }
                })
                .collect_view()}
        </div>
    }
}

/// A card in a lane: what it is and where it stands, nothing to drag.
#[component]
fn LaneCard(root: String, ticket: TicketSummary) -> impl IntoView {
    let ctx = Ctx::get();
    let open = {
        let (root, column, epic, file) = (
            root.clone(),
            ticket.column,
            ticket.epic.clone(),
            ticket.file.clone(),
        );
        move || {
            let (root, epic, file) = (root.clone(), epic.clone(), file.clone());
            spawn_local(async move {
                match api::read_lane_ticket(&root, column, epic.as_deref(), &file).await {
                    Ok(ticket) => ctx.overlay.set(Some(Overlay::LaneTicket {
                        root,
                        ticket: Box::new(ticket),
                    })),
                    Err(e) => ctx.error.set(Some(e)),
                }
            });
        }
    };
    let open_key = open.clone();
    let epic = ticket
        .epic
        .clone()
        .or_else(|| (!ticket.epic_label.is_empty()).then(|| ticket.epic_label.clone()));
    view! {
        <li
            tabindex="0"
            class="cursor-pointer rounded-md border border-slate-200 bg-white p-2 text-sm shadow-sm hover:border-accent-400 focus:outline-2 focus:outline-accent-500 dark:border-slate-700 dark:bg-slate-900"
            title="View only: click to read it"
            on:click=move |_| open()
            on:keydown=move |ev| {
                if ev.key() == "Enter" {
                    open_key();
                }
            }
        >
            <div class="flex items-baseline gap-2">
                <span class="font-mono text-xs text-slate-500">{ticket.id.clone()}</span>
                <span class="flex-1 font-medium">{ticket.title.clone()}</span>
            </div>
            <div class="mt-1 flex flex-wrap items-center gap-1 text-xs">
                {match (ticket.lock.clone(), ticket.lapsed.clone()) {
                    (Some(l), _) => Some(view! { <LockBadge lock=l lapsed=false /> }),
                    (None, Some(l)) => Some(view! { <LockBadge lock=l lapsed=true /> }),
                    (None, None) => None,
                }}
                {epic
                    .map(|name| {
                        view! {
                            <span class="max-w-40 truncate rounded bg-slate-500 px-1.5 font-medium text-white" title=format!("Epic or sprint: {name}")>
                                {name.clone()}
                            </span>
                        }
                    })}
                {(!ticket.points.trim().is_empty())
                    .then(|| {
                        view! {
                            <span class="rounded bg-slate-200 px-1.5 font-mono text-slate-700 dark:bg-slate-700 dark:text-slate-200">
                                {format!("{} pts", ticket.points.trim())}
                            </span>
                        }
                    })}
                {(!ticket.priority.is_empty())
                    .then(|| {
                        view! {
                            <span class=format!("rounded px-1.5 font-medium {}", priority_tone(&ticket.priority))>
                                {ticket.priority.clone()}
                            </span>
                        }
                    })}
                {(!ticket.assignee.is_empty())
                    .then(|| view! { <span class="text-slate-500">"@" {ticket.assignee.clone()}</span> })}
                {ticket
                    .tags
                    .iter()
                    .map(|tag| {
                        view! {
                            <span class="rounded bg-accent-100 px-1.5 uppercase text-accent-800 dark:bg-accent-900 dark:text-accent-200">
                                {tag.clone()}
                            </span>
                        }
                    })
                    .collect_view()}
            </div>
        </li>
    }
}

/// A lane's ticket, read-only, with the way to its own board.
#[component]
pub fn LaneTicketPanel(root: String, ticket: LaneTicket) -> impl IntoView {
    let ctx = Ctx::get();
    let t = ticket.ticket;
    let place = amd_ui_model::waiting::place(&t);
    let target = (t.column, t.epic.clone(), t.file.clone());
    let open_there = move |_| {
        // Where the ticket is now: it may have moved since it was opened,
        // and the lanes follow the files.
        let (column, epic, file) = ctx
            .lanes
            .with_untracked(|lanes| {
                lanes
                    .iter()
                    .filter(|lane| lane.root == root)
                    .flat_map(|lane| lane.columns.iter().flat_map(|c| &c.tickets))
                    .find(|t| t.file == target.2)
                    .map(|t| (t.column, t.epic.clone(), t.file.clone()))
            })
            .unwrap_or_else(|| target.clone());
        ctx.close_overlay();
        crate::home::open(
            ctx,
            root.clone(),
            View::Board,
            Some((String::new(), column, epic, file)),
        );
    };
    let field = |label: &'static str, value: String| {
        (!value.trim().is_empty()).then(|| {
            view! {
                <div class="flex gap-2 text-sm">
                    <span class="w-20 shrink-0 text-slate-500">{label}</span>
                    <span>{value}</span>
                </div>
            }
        })
    };
    view! {
        <Modal title=format!("{} {}", t.id, t.title) width="max-w-3xl">
            <p class="rounded bg-slate-100 px-3 py-2 text-sm dark:bg-slate-800">
                {format!("View only: a ticket on {}'s board. ", ticket.project)}
                "Open it on its board to change it."
            </p>
            <div class="flex flex-col gap-1">
                {field("Project", ticket.project.clone())}
                {field("Where", place)}
                {field("Priority", t.priority.clone())}
                {field("Points", t.points.clone())}
                {field("Assignee", t.assignee.clone())}
                {field("Repository", t.repository.clone())}
                {field("Tags", t.tags.join(", "))}
                {t.lock.clone().map(|l| view! { <div class="text-sm"><LockBadge lock=l lapsed=false /></div> })}
            </div>
            <div
                class="doc min-h-0 overflow-y-auto"
                inner_html=ticket.html
                // Links inside go nowhere from here.
                on:click=|ev| ev.prevent_default()
            ></div>
            <div class="flex justify-end gap-2">
                <button
                    class="rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-700"
                    on:click=open_there
                >
                    "Open on its board"
                </button>
                <button
                    class="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                    on:click=move |_| ctx.close_overlay()
                >
                    "Close"
                </button>
            </div>
        </Modal>
    }
}
