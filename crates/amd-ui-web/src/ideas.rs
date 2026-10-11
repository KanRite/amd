//! The Ideas view (ticket 131): the project's ideas, best first, each with
//! your − and + (up to three either way), and one opened beside the list
//! with its write-up, who voted, the notes, and Make a ticket.

use amd_ui_model::epic::Place;
use amd_ui_model::{Column, IdeaCard, IdeaDetail, IdeaList};
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;

use crate::api;
use crate::app::{Ctx, View};

const SMALL: &str = "rounded border border-slate-300 px-2 py-0.5 text-xs hover:bg-slate-100 disabled:opacity-40 dark:border-slate-600 dark:hover:bg-slate-700";
const PRIMARY: &str = "rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50";
const MAX: i32 = 3;

fn signed(v: i32) -> String {
    if v > 0 {
        format!("+{v}")
    } else {
        v.to_string()
    }
}

/// Green above nothing, red below.
fn tone(score: i32) -> &'static str {
    match score {
        s if s > 0 => "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-100",
        s if s < 0 => "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-100",
        _ => "bg-slate-100 text-slate-600 dark:bg-slate-700 dark:text-slate-300",
    }
}

#[component]
pub fn IdeasView() -> impl IntoView {
    let ctx = Ctx::get();
    let list = RwSignal::new(None::<Result<IdeaList, String>>);
    let open = RwSignal::new(None::<String>);
    let detail = RwSignal::new(None::<Result<IdeaDetail, String>>);
    let adding = RwSignal::new(false);
    let show_archived = RwSignal::new(false);
    let busy = RwSignal::new(false);
    // The ticket just made from the open idea, kept across its reload.
    let made = RwSignal::new(None::<amd_ui_model::IdeaTicket>);

    let reload = move || {
        spawn_local(async move {
            list.set(Some(api::list_ideas().await));
            if let Some(path) = open.get_untracked() {
                detail.set(Some(api::read_idea(&path).await));
            }
        });
    };
    reload();
    let select = move |path: String| {
        made.set(None);
        open.set(Some(path.clone()));
        adding.set(false);
        spawn_local(async move { detail.set(Some(api::read_idea(&path).await)) });
    };
    let vote = move |path: String, delta: i32| {
        busy.set(true);
        spawn_local(async move {
            match api::vote_idea(&path, delta).await {
                Ok(change) => ctx.report(change.sync),
                Err(e) => ctx.error.set(Some(e)),
            }
            busy.set(false);
            reload();
        });
    };

    let done = move |path: Option<String>| {
        adding.set(false);
        if let Some(path) = path {
            select(path);
        }
        reload();
    };

    let card = move |idea: IdeaCard| {
        let path = idea.path.clone();
        let (up, down, pick) = (path.clone(), path.clone(), path.clone());
        let selected = Memo::new(move |_| open.get().as_deref() == Some(path.as_str()));
        let mine = idea.mine;
        view! {
            <li
                class="flex items-center gap-3 rounded-md border bg-white px-3 py-2 text-sm dark:bg-slate-800"
                class=("border-accent-500", move || selected.get())
                class=("border-slate-200", move || !selected.get())
                class=("dark:border-slate-700", move || !selected.get())
            >
                <span
                    class=format!("w-10 shrink-0 rounded px-1.5 py-0.5 text-center font-mono text-sm font-semibold {}", tone(idea.score))
                    title=format!("score: {} from {} vote{}", signed(idea.score), idea.voters, if idea.voters == 1 { "" } else { "s" })
                >
                    {signed(idea.score)}
                </span>
                <button
                    type="button"
                    class="flex min-w-0 flex-1 flex-col text-left"
                    on:click=move |_| select(pick.clone())
                >
                    <span class="truncate font-medium" class=("italic", idea.archived) class=("text-slate-500", idea.archived)>
                        {idea.title.clone()}
                    </span>
                    <span class="truncate text-xs text-slate-500">
                        {[
                            idea.author.clone(),
                            (idea.notes > 0)
                                .then(|| format!("{} note{}", idea.notes, if idea.notes == 1 { "" } else { "s" })),
                            (!idea.tickets.is_empty()).then(|| format!("→ {}", idea.tickets.join(", "))),
                        ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · ")}
                    </span>
                </button>
                <div class="flex shrink-0 items-center gap-1" title="Your vote: up to three either way">
                    <button
                        type="button"
                        class=SMALL
                        aria-label="Vote down"
                        disabled=move || { busy.get() || mine <= -MAX }
                        on:click=move |_| vote(down.clone(), -1)
                    >
                        "−"
                    </button>
                    <span class="w-7 text-center font-mono text-xs" class=("text-slate-400", mine == 0)>
                        {if mine == 0 { "·".to_string() } else { signed(mine) }}
                    </span>
                    <button
                        type="button"
                        class=SMALL
                        aria-label="Vote up"
                        disabled=move || { busy.get() || mine >= MAX }
                        on:click=move |_| vote(up.clone(), 1)
                    >
                        "+"
                    </button>
                </div>
            </li>
        }
    };

    view! {
        <div class="flex h-full min-h-0 gap-4 p-4">
            <section class="flex min-h-0 w-1/2 flex-col gap-3">
                <div class="flex items-center gap-3">
                    <h2 class="text-lg font-semibold">"Ideas"</h2>
                    <span class="flex-1 text-xs text-slate-500">
                        "Put an idea forward, write it up, and vote: up to three + or − each. The best rise to the top."
                    </span>
                    <button
                        class=PRIMARY
                        disabled=move || !list.with(|l| matches!(l, Some(Ok(l)) if l.cloned))
                        on:click=move |_| {
                            adding.set(true);
                            open.set(None);
                        }
                    >
                        "New idea"
                    </button>
                </div>
                <div class="min-h-0 flex-1 overflow-y-auto">
                    {move || match list.get() {
                        None => view! { <p class="text-sm text-slate-500">"Loading…"</p> }.into_any(),
                        Some(Err(e)) => view! { <p class="text-sm text-red-700">{e}</p> }.into_any(),
                        Some(Ok(l)) if !l.cloned => view! {
                            <p class="text-sm text-slate-500">{format!("{} isn't cloned on this machine.", l.source)}</p>
                        }.into_any(),
                        Some(Ok(l)) if l.ideas.is_empty() => view! {
                            <p class="text-sm text-slate-500">"No ideas yet. New idea puts the first one forward."</p>
                        }.into_any(),
                        Some(Ok(l)) => {
                            let (archived, current): (Vec<_>, Vec<_>) = l.ideas.into_iter().partition(|i| i.archived);
                            let count = archived.len();
                            view! {
                                <ul class="flex flex-col gap-1.5">{current.into_iter().map(card).collect_view()}</ul>
                                <Show when=move || { count > 0 }>
                                    <button
                                        type="button"
                                        class="mt-2 text-sm text-slate-500 hover:underline"
                                        on:click=move |_| show_archived.update(|s| *s = !*s)
                                    >
                                        {move || format!("{} Archived ({count})", if show_archived.get() { "▾" } else { "▸" })}
                                    </button>
                                </Show>
                                <Show when=move || show_archived.get()>
                                    <ul class="mt-1 flex flex-col gap-1.5">{archived.clone().into_iter().map(card).collect_view()}</ul>
                                </Show>
                            }.into_any()
                        }
                    }}
                </div>
            </section>
            <section class="min-h-0 w-1/2 overflow-y-auto rounded-md border border-slate-200 bg-white p-4 dark:border-slate-700 dark:bg-slate-800">
                {move || {
                    if adding.get() {
                        return view! { <NewIdea on_done=done /> }.into_any();
                    }
                    match detail.get() {
                        _ if open.with(Option::is_none) => view! {
                            <p class="text-sm text-slate-500">"Open an idea to read it, see who voted and add a note."</p>
                        }.into_any(),
                        None => view! { <p class="text-sm text-slate-500">"Loading…"</p> }.into_any(),
                        Some(Err(e)) => view! { <p class="text-sm text-red-700">{e}</p> }.into_any(),
                        Some(Ok(d)) => view! { <Detail detail=d reload=reload made=made /> }.into_any(),
                    }
                }}
            </section>
        </div>
    }
}

/// An opened idea.
#[component]
fn Detail(
    detail: IdeaDetail,
    reload: impl Fn() + Copy + Send + Sync + 'static,
    made: RwSignal<Option<amd_ui_model::IdeaTicket>>,
) -> impl IntoView {
    let ctx = Ctx::get();
    let note = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let path = detail.card.path.clone();
    let (note_path, ticket_path, doc_path) = (path.clone(), path.clone(), path.clone());
    let source = detail.source.clone();

    let add_note = move |_| {
        let text = note.get_untracked();
        let path = note_path.clone();
        busy.set(true);
        spawn_local(async move {
            match api::note_idea(&path, &text).await {
                Ok(change) => {
                    ctx.report(change.sync);
                    note.set(String::new());
                    reload();
                }
                Err(e) => ctx.error.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let make_ticket = move |_| {
        let path = ticket_path.clone();
        busy.set(true);
        spawn_local(async move {
            match api::idea_to_ticket(&path).await {
                Ok(ticket) => {
                    ctx.report(ticket.sync.clone());
                    made.set(Some(ticket));
                    ctx.reload();
                    reload();
                }
                Err(e) => ctx.error.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let open_ticket = move |_| {
        if let Some(t) = made.get_untracked() {
            ctx.view.set(View::Board);
            ctx.open_ticket(
                Place {
                    repo: t.repo,
                    column: Column::Backlog,
                    epic: None,
                },
                t.file,
            );
        }
    };
    // Edited like any document (ticket 176).
    let edit = move |_| {
        ctx.doc_scope.set(Some(amd_ui_model::DocScope {
            shared: false,
            folder: "ideas".to_string(),
        }));
        ctx.document.set(Some((source.clone(), doc_path.clone())));
        ctx.view.set(View::Documents);
    };
    // Links mustn't navigate the window: web links open outside.
    let on_click = move |ev: leptos::ev::MouseEvent| {
        let Some(anchor) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .and_then(|e| e.closest("a").ok().flatten())
        else {
            return;
        };
        let href = anchor.get_attribute("href").unwrap_or_default();
        ev.prevent_default();
        if href.starts_with("http://")
            || href.starts_with("https://")
            || href.starts_with("mailto:")
        {
            spawn_local(async move {
                if let Err(e) = api::open_external(&href).await {
                    ctx.error.set(Some(e));
                }
            });
        }
    };

    let card = detail.card.clone();
    let votes = detail
        .votes
        .iter()
        .map(|v| format!("{} {}", v.name, signed(v.value)))
        .collect::<Vec<_>>()
        .join(" · ");
    view! {
        <article class="flex flex-col gap-3">
            <header class="flex items-start gap-3">
                <span class=format!("rounded px-2 py-0.5 font-mono text-lg font-semibold {}", tone(card.score))>{signed(card.score)}</span>
                <div class="flex flex-1 flex-col">
                    <h2 class="text-xl font-semibold">{card.title.clone()}</h2>
                    <p class="text-xs text-slate-500">
                        {[
                            Some(card.author.clone().map(|a| format!("Put forward by {a}")).unwrap_or_else(|| "Anonymous".to_string())),
                            (!card.created.is_empty()).then(|| card.created.clone()),
                            Some(card.path.clone()),
                        ]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>()
                            .join(" · ")}
                    </p>
                </div>
                <button type="button" class=SMALL on:click=edit title="Open it in Documents, where Edit changes the write-up">"Open in Documents"</button>
            </header>
            {(card.archived).then(|| view! { <p class="rounded bg-slate-100 px-3 py-1 text-xs dark:bg-slate-700">"Archived"</p> })}
            {if detail.html.trim().is_empty() {
                view! { <p class="text-sm italic text-slate-500">"No write-up yet. Open in Documents, then Edit, to add one."</p> }.into_any()
            } else {
                view! { <div class="doc text-sm" inner_html=detail.html.clone() on:click=on_click></div> }.into_any()
            }}
            <section class="flex flex-col gap-1 border-t border-slate-200 pt-2 dark:border-slate-700">
                <h3 class="text-sm font-semibold">"Votes"</h3>
                <p class="text-sm">{if votes.is_empty() { "None yet.".to_string() } else { votes }}</p>
            </section>
            <section class="flex flex-col gap-2 border-t border-slate-200 pt-2 dark:border-slate-700">
                <h3 class="text-sm font-semibold">"Notes"</h3>
                {if detail.notes.is_empty() {
                    view! { <p class="text-sm text-slate-500">"No notes yet."</p> }.into_any()
                } else {
                    view! {
                        <ul class="flex flex-col gap-2">
                            {detail.notes.iter().map(|n| view! {
                                <li class="rounded bg-slate-50 px-3 py-2 text-sm dark:bg-slate-900">
                                    <p class="text-xs text-slate-500">{format!("{} · {}", n.name, n.date)}</p>
                                    <p class="whitespace-pre-wrap">{n.text.clone()}</p>
                                </li>
                            }).collect_view()}
                        </ul>
                    }.into_any()
                }}
                <textarea
                    aria-label="A note"
                    placeholder="Add a note"
                    rows="2"
                    class="rounded border border-slate-300 bg-slate-50 px-2 py-1.5 text-sm dark:border-slate-600 dark:bg-slate-900"
                    prop:value=move || note.get()
                    on:input=move |ev| note.set(event_target_value(&ev))
                ></textarea>
                <div class="flex justify-end">
                    <button class=PRIMARY disabled=move || busy.get() || note.with(|n| n.trim().is_empty()) on:click=add_note>
                        "Add note"
                    </button>
                </div>
            </section>
            <section class="flex flex-wrap items-center gap-2 border-t border-slate-200 pt-2 dark:border-slate-700">
                <span class="flex-1 text-xs text-slate-500">
                    {if card.tickets.is_empty() {
                        "Ready to do it? Make a backlog ticket from it, linked both ways.".to_string()
                    } else {
                        format!("Tickets: {}", card.tickets.join(", "))
                    }}
                </span>
                <Show when=move || made.with(Option::is_some)>
                    <button type="button" class=SMALL on:click=open_ticket>
                        {move || made.with(|t| t.as_ref().map(|t| format!("Open ticket {}", t.id)).unwrap_or_default())}
                    </button>
                </Show>
                <button class=PRIMARY disabled=move || busy.get() on:click=make_ticket>"Make a ticket"</button>
            </section>
        </article>
    }
}

/// Putting an idea forward: a title, a write-up in rich text, and whether
/// your name goes on it.
#[component]
fn NewIdea(on_done: impl Fn(Option<String>) + Copy + Send + Sync + 'static) -> impl IntoView {
    use amd_ui_model::rich::to_markdown;
    let ctx = Ctx::get();
    let title = RwSignal::new(String::new());
    let named = RwSignal::new(true);
    let busy = RwSignal::new(false);
    let editor = NodeRef::<leptos::html::Div>::new();
    let title_input = NodeRef::<leptos::html::Input>::new();
    Effect::new(move |_| {
        if let Some(input) = title_input.get() {
            let _ = input.focus();
        }
    });
    let create = move |_| {
        let body = editor
            .get_untracked()
            .map(|el| to_markdown(&crate::rich_dom::read_blocks(&el)))
            .unwrap_or_default();
        let t = title.get_untracked();
        let named = named.get_untracked();
        busy.set(true);
        spawn_local(async move {
            match api::create_idea(&t, &body, named).await {
                Ok(change) => {
                    ctx.report(change.sync);
                    on_done(change.path);
                }
                Err(e) => ctx.error.set(Some(e)),
            }
            busy.set(false);
        });
    };
    view! {
        <div class="flex flex-col gap-3">
            <h2 class="text-lg font-semibold">"New idea"</h2>
            <input
                node_ref=title_input
                aria-label="Title"
                placeholder="What's the idea?"
                class="rounded border border-slate-300 bg-slate-50 px-2 py-1.5 text-sm dark:border-slate-600 dark:bg-slate-900"
                prop:value=move || title.get()
                on:input=move |ev| title.set(event_target_value(&ev))
            />
            <crate::rich_editor::Toolbar on_change=|| {} />
            <div
                node_ref=editor
                contenteditable="true"
                role="textbox"
                aria-multiline="true"
                aria-label="Write-up"
                class="rich-editor doc min-h-[30vh] rounded border border-slate-300 bg-white p-3 text-sm focus:outline-2 focus:outline-accent-500 dark:border-slate-600 dark:bg-slate-900"
                on:focus=move |_| crate::rich_dom::use_paragraphs()
            ></div>
            <label class="flex items-center gap-2 text-sm">
                <input type="checkbox" prop:checked=move || named.get() on:change=move |ev| named.set(event_target_checked(&ev)) />
                "Put my name on it"
            </label>
            <div class="flex justify-end gap-2">
                <button type="button" class=SMALL on:click=move |_| on_done(None)>"Cancel"</button>
                <button class=PRIMARY disabled=move || busy.get() || title.with(|t| t.trim().is_empty()) on:click=create>
                    "Put it forward"
                </button>
            </div>
        </div>
    }
}
