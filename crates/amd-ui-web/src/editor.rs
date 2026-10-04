//! The ticket editor. It opens in rich-text mode — frontmatter as a form,
//! the body as formatted text — and a switch flips to the raw markdown.
//!
//! Whatever the user did not touch is saved byte-for-byte as it was: an
//! unedited body keeps its original markdown, and only changed frontmatter
//! lines are rewritten (see `amd_ui_model::ticket::apply_fields`).

use amd_ui_model::epic::Place;
use amd_ui_model::rich::{parse_markdown, to_html, to_markdown};
use amd_ui_model::ticket::{
    self, BRANCH_TYPES, DEFAULT_PRIORITY, Fields, PRIORITIES, VALID_POINTS,
};
use amd_ui_model::{TicketDocument, parse_frontmatter};
use leptos::html::Div;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;

use crate::api;
use crate::app::{Ctx, Modal};
use crate::rich_dom;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Rich,
    Markdown,
}

#[component]
pub fn Editor(doc: TicketDocument) -> impl IntoView {
    let ctx = Ctx::get();
    let TicketDocument {
        repo,
        file,
        column,
        epic,
        content,
        lock,
        version,
    } = doc;
    let path = match &epic {
        Some(name) => format!("{}/{name}/{file}", column.dir_name()),
        None => format!("{}/{file}", column.dir_name()),
    };
    let place = StoredValue::new(Place { repo, column, epic });
    let lock_file = StoredValue::new(file.clone());
    // The ticket's lock, kept current as it is locked and unlocked here.
    let lock = RwSignal::new(lock);
    // The fingerprint of the ticket as it was opened (or last locked or
    // unlocked here); saving hands it back.
    let version = RwSignal::new(version);
    // Someone else holds it: it can be read, but not saved or archived.
    let held = move || lock.with(|l| l.as_ref().is_some_and(|l| !l.mine));
    let original = StoredValue::new(content.clone());

    let mode = RwSignal::new(Mode::Rich);
    // Markdown mode edits this; rich mode was rendered from it.
    let text = RwSignal::new(content.clone());
    let fm = parse_frontmatter(&content);
    let has_frontmatter = RwSignal::new(ticket::split(&content).frontmatter.is_some());
    let fields = RwSignal::new(Fields::from(&fm));
    let tags_text = RwSignal::new(fm.tags.join(", "));
    let id = RwSignal::new(fm.id);
    let body_touched = RwSignal::new(false);
    let dirty = RwSignal::new(false);
    let saving = RwSignal::new(false);
    // Why the last save was refused (an invalid priority, a write error).
    let save_error = RwSignal::new(None::<String>);
    let editor = NodeRef::<Div>::new();

    let render_rich = move || {
        if let Some(el) = editor.get_untracked() {
            let body = text.with_untracked(|t| ticket::split(t).body.to_string());
            el.set_inner_html(&to_html(&parse_markdown(&body)));
        }
    };
    // Fill the rich area once it is in the DOM.
    Effect::new(move |_| {
        if editor.get().is_some() {
            rich_dom::use_paragraphs();
            render_rich();
        }
    });

    // The ticket as it would be saved right now.
    let current = move || -> String {
        let base = text.get_untracked();
        if mode.get_untracked() == Mode::Markdown {
            return base;
        }
        let parts = ticket::split(&base);
        let body = match (body_touched.get_untracked(), editor.get_untracked()) {
            (true, Some(el)) => {
                let md = to_markdown(&rich_dom::read_blocks(&el));
                // Keep the blank line amd puts between frontmatter and body.
                if parts.frontmatter.is_some() {
                    format!("\n{md}")
                } else {
                    md
                }
            }
            _ => parts.body.to_string(),
        };
        let joined = format!("{}{body}", parts.frontmatter.unwrap_or(""));
        fields.with_untracked(|f| ticket::apply_fields(&joined, f))
    };
    let refresh_dirty = move || {
        save_error.set(None);
        dirty.set(original.with_value(|o| *o != current()));
    };
    let touch_body = move || {
        body_touched.set(true);
        refresh_dirty();
    };

    let toggle_mode = move |_| match mode.get_untracked() {
        Mode::Rich => {
            text.set(current());
            mode.set(Mode::Markdown);
        }
        Mode::Markdown => {
            let md = text.get_untracked();
            let fm = parse_frontmatter(&md);
            has_frontmatter.set(ticket::split(&md).frontmatter.is_some());
            tags_text.set(fm.tags.join(", "));
            id.set(fm.id.clone());
            fields.set(Fields::from(&fm));
            body_touched.set(false);
            mode.set(Mode::Rich);
            render_rich();
        }
    };

    // Archiving asks once more before taking the ticket off the board.
    let confirm_archive = RwSignal::new(false);
    let archive = {
        let file = file.clone();
        move |_| {
            if !confirm_archive.get_untracked() {
                confirm_archive.set(true);
                return;
            }
            let file = file.clone();
            saving.set(true);
            spawn_local(async move {
                match api::archive_ticket(&place.get_value(), &file).await {
                    Ok(report) => {
                        ctx.report(report);
                        ctx.close_overlay();
                        ctx.reload();
                    }
                    Err(e) => save_error.set(Some(e)),
                }
                saving.set(false);
            });
        }
    };

    let save = move |_| {
        let file = file.clone();
        // Same rule the backend applies; checking here keeps the editor open
        // with the reason instead of a round trip.
        let content = match ticket::ensure_priority(&current()) {
            Ok(content) => content,
            Err(e) => return save_error.set(Some(e)),
        };
        saving.set(true);
        spawn_local(async move {
            let opened = version.get_untracked();
            match api::save_ticket(&place.get_value(), &file, &content, &opened).await {
                Ok(report) => {
                    ctx.report(report);
                    ctx.close_overlay();
                    ctx.reload();
                }
                Err(e) => save_error.set(Some(e)),
            }
            saving.set(false);
        });
    };

    let set_field = move |f: fn(&mut Fields, String)| {
        move |ev| {
            let value = event_target_value(&ev);
            fields.update(|fields| f(fields, value));
            refresh_dirty();
        }
    };

    let is_rich = move || mode.get() == Mode::Rich;
    let input = "rounded border border-slate-300 bg-slate-50 px-2 py-1 text-sm dark:border-slate-600 dark:bg-slate-900";
    let label = "flex flex-col gap-1 text-xs font-medium text-slate-600 dark:text-slate-300";

    view! {
        <Modal title=format!("Edit {path}") width="max-w-4xl">
            <LockBar lock version place file=lock_file />
            // Everything above the buttons scrolls as one, so a short window
            // scrolls the dialog instead of squeezing the editor to nothing.
            <div class="-mx-1 flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-1">
            // ---- Frontmatter as a form (rich mode only)
            <Show when=move || is_rich() && has_frontmatter.get()>
                <div class="grid grid-cols-1 gap-2 sm:grid-cols-[1fr_12rem]">
                    <label class=label>
                        "Title"
                        <input
                            type="text"
                            class=input
                            prop:value=move || fields.with(|f| f.title.clone())
                            on:input=set_field(|f, v| f.title = v)
                        />
                    </label>
                    <label class=label>
                        "Ticket"
                        <input type="text" class=format!("{input} opacity-70") readonly prop:value=move || id.get() />
                    </label>
                    <label class=label>
                        "Assignee"
                        <input
                            type="text"
                            class=input
                            prop:value=move || fields.with(|f| f.assignee.clone())
                            on:input=set_field(|f, v| f.assignee = v)
                        />
                    </label>
                    <label class=label>
                        "Branch type"
                        <select
                            class=input
                            prop:value=move || fields.with(|f| f.branch_type.clone())
                            on:change=set_field(|f, v| f.branch_type = v)
                        >
                            <option value="">"none"</option>
                            {BRANCH_TYPES.map(|t| view! { <option value=t>{t}</option> }).collect_view()}
                        </select>
                    </label>
                    <label class=label>
                        "Tags (comma separated)"
                        <input
                            type="text"
                            class=input
                            prop:value=move || tags_text.get()
                            on:input=move |ev| {
                                let value = event_target_value(&ev);
                                fields.update(|f| f.tags = ticket::parse_tags(&value));
                                tags_text.set(value);
                                refresh_dirty();
                            }
                        />
                    </label>
                    <label class=label>
                        "Points"
                        <select
                            class=input
                            title="Size estimate; a ticket needs points to go into a sprint"
                            prop:value=move || fields.with(|f| f.points.clone())
                            on:change=set_field(|f, v| f.points = v)
                        >
                            <option value="">"unsized"</option>
                            {VALID_POINTS.map(|p| view! { <option value=p>{p}</option> }).collect_view()}
                            // A value amd doesn't accept, from markdown mode or an old
                            // ticket, stays visible so the user can see what to correct.
                            {move || {
                                fields.with(|f| {
                                    let p = f.points.trim();
                                    (!p.is_empty() && !VALID_POINTS.contains(&p))
                                        .then(|| {
                                            let bad = p.to_string();
                                            view! {
                                                <option value=bad.clone() disabled>
                                                    {format!("{bad} (not valid)")}
                                                </option>
                                            }
                                        })
                                })
                            }}
                        </select>
                    </label>
                    <label class=label>
                        "Priority"
                        <select
                            class=input
                            prop:value=move || {
                                fields.with(|f| {
                                    if f.priority.is_empty() { DEFAULT_PRIORITY.to_string() } else { f.priority.clone() }
                                })
                            }
                            on:change=set_field(|f, v| f.priority = v)
                        >
                            {PRIORITIES.map(|p| view! { <option value=p>{p}</option> }).collect_view()}
                            // An invalid value typed in Markdown mode stays visible, so the
                            // user can see what to correct.
                            {move || {
                                fields.with(|f| {
                                    (!f.priority.is_empty() && !PRIORITIES.contains(&f.priority.as_str()))
                                        .then(|| {
                                            let bad = f.priority.clone();
                                            view! {
                                                <option value=bad.clone() disabled>
                                                    {format!("{bad} (not valid)")}
                                                </option>
                                            }
                                        })
                                })
                            }}
                        </select>
                    </label>
                </div>
            </Show>

            // ---- Rich text: toolbar + editable body
            <div class=move || if is_rich() { "flex shrink-0 flex-col gap-2" } else { "hidden" }>
                <Toolbar on_change=touch_body />
                <div
                    node_ref=editor
                    contenteditable="true"
                    role="textbox"
                    aria-multiline="true"
                    aria-label="Ticket text"
                    class="rich-editor h-[45vh] min-h-48 overflow-y-auto rounded border border-slate-300 bg-slate-50 p-3 text-sm focus:outline-2 focus:outline-accent-500 dark:border-slate-600 dark:bg-slate-900"
                    on:input=move |_| touch_body()
                    on:click=move |ev| {
                        let target = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok());
                        if let Some(el) = target {
                            // Links must not navigate the app's window away.
                            if el.closest("a").ok().flatten().is_some() {
                                ev.prevent_default();
                            }
                            // Ticking a checklist box is an edit too.
                            if el.tag_name().eq_ignore_ascii_case("input") {
                                touch_body();
                            }
                        }
                    }
                    on:paste=move |ev| {
                        // Paste as plain text so foreign markup never enters the ticket.
                        ev.prevent_default();
                        let text = ev.clipboard_data().and_then(|d| d.get_data("text/plain").ok());
                        if let Some(text) = text {
                            rich_dom::exec("insertText", Some(&text));
                        }
                    }
                ></div>
            </div>

            // ---- Raw markdown
            <textarea
                aria-label="Ticket markdown"
                class=move || {
                    format!(
                        "h-[60vh] min-h-48 w-full shrink-0 resize-none rounded border border-slate-300 bg-slate-50 p-3 font-mono text-sm dark:border-slate-600 dark:bg-slate-900 {}",
                        if is_rich() { "hidden" } else { "" },
                    )
                }
                spellcheck="false"
                prop:value=move || text.get()
                on:input=move |ev| {
                    text.set(event_target_value(&ev));
                    refresh_dirty();
                }
            ></textarea>
            </div>

            <div class="flex shrink-0 flex-wrap items-center justify-end gap-3">
                <button
                    type="button"
                    title="Take this ticket off the board into tasks/archive/"
                    class=move || {
                        if confirm_archive.get() {
                            "rounded bg-red-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-red-500 disabled:opacity-50"
                        } else {
                            "rounded border border-slate-300 px-3 py-1.5 text-sm text-red-700 hover:bg-red-50 disabled:opacity-50 dark:border-slate-600 dark:text-red-300 dark:hover:bg-red-950"
                        }
                    }
                    disabled=move || saving.get() || held()
                    on:click=archive
                    on:blur=move |_| confirm_archive.set(false)
                >
                    {move || if confirm_archive.get() { "Confirm archive" } else { "Archive" }}
                </button>
                <span class="flex-1 text-xs">
                    {move || match save_error.get() {
                        Some(e) => {
                            view! {
                                <span role="alert" class="font-medium text-red-700 dark:text-red-300">
                                    {e}
                                </span>
                            }
                                .into_any()
                        }
                        None => {
                            view! {
                                <span class="text-slate-500">
                                    {if dirty.get() { "Unsaved changes" } else { "" }}
                                </span>
                            }
                                .into_any()
                        }
                    }}
                </span>
                <button
                    type="button"
                    role="switch"
                    aria-checked=move || (!is_rich()).to_string()
                    title=move || {
                        if is_rich() { "Edit the raw markdown" } else { "Back to rich text" }
                    }
                    class="flex items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-slate-100 dark:hover:bg-slate-700"
                    on:click=toggle_mode
                >
                    <span class=move || {
                        format!(
                            "relative inline-flex h-5 w-9 shrink-0 rounded-full transition {}",
                            if is_rich() { "bg-slate-300 dark:bg-slate-600" } else { "bg-accent-600" },
                        )
                    }>
                        <span class=move || {
                            format!(
                                "absolute top-0.5 h-4 w-4 rounded-full bg-white shadow transition-all {}",
                                if is_rich() { "left-0.5" } else { "left-4.5" },
                            )
                        }></span>
                    </span>
                    {move || if is_rich() { "Markdown" } else { "Rich Text" }}
                </button>
                <button
                    class="rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                    on:click=move |_| ctx.close_overlay()
                >
                    "Close"
                </button>
                <button
                    class="rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50"
                    disabled=move || saving.get() || !dirty.get() || held()
                    title=move || held().then_some("Locked by someone else")
                    on:click=save
                >
                    "Save"
                </button>
            </div>
        </Modal>
    }
}

/// The ticket's lock, above the editor: lock it, renew or release your own,
/// or — a separate, confirmed step — break someone else's.
#[component]
fn LockBar(
    lock: RwSignal<Option<amd_ui_model::Lock>>,
    version: RwSignal<String>,
    place: StoredValue<Place>,
    file: StoredValue<String>,
) -> impl IntoView {
    let ctx = Ctx::get();
    let busy = RwSignal::new(false);
    let problem = RwSignal::new(None::<String>);
    let confirm_break = RwSignal::new(false);
    let hours = move || {
        let repo = place.with_value(|p| p.repo.clone());
        ctx.board.with(|b| {
            b.as_ref()
                .and_then(|b| b.repos.iter().find(|r| r.name == repo))
                .map_or(amd_ui_model::DEFAULT_LEASE_HOURS, |r| r.lease_hours)
        })
    };

    // Runs a lock change, then reads the lock back from the ticket.
    let act = move |action: &'static str| {
        busy.set(true);
        problem.set(None);
        confirm_break.set(false);
        spawn_local(async move {
            let (p, f) = (place.get_value(), file.get_value());
            let done = match action {
                "lock" => api::lock_ticket(&p, &f).await,
                "unlock" => api::unlock_ticket(&p, &f, false).await,
                _ => api::unlock_ticket(&p, &f, true).await,
            };
            match done {
                Ok(report) => {
                    ctx.report(report);
                    if let Ok(doc) = api::open_ticket(&p, &f).await {
                        lock.set(doc.lock);
                        version.set(doc.version);
                    }
                    ctx.reload();
                }
                Err(e) => problem.set(Some(e)),
            }
            busy.set(false);
        });
    };
    let button = "rounded border border-slate-300 px-2 py-1 text-xs hover:bg-slate-100 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700";

    view! {
        <div class=move || {
            let tone = match lock.get() {
                Some(l) if !l.mine => "bg-amber-100 text-amber-900 dark:bg-amber-900/60 dark:text-amber-100",
                _ => "bg-slate-100 text-slate-700 dark:bg-slate-900 dark:text-slate-300",
            };
            format!("flex shrink-0 flex-wrap items-center gap-2 rounded px-3 py-1.5 text-sm {tone}")
        }>
            <span class="flex-1">
                {move || match lock.get() {
                    None => "Not locked.".to_string(),
                    Some(l) if l.mine => format!("Locked by you until {}.", l.until_local),
                    Some(l) => {
                        format!(
                            "Locked by {} until {}. You can read it, but only they can change it.",
                            l.by,
                            l.until_local,
                        )
                    }
                }}
            </span>
            {move || problem.get().map(|p| view! { <span role="alert" class="font-medium text-red-700 dark:text-red-300">{p}</span> })}
            {move || match lock.get() {
                None => {
                    view! {
                        <button class=button disabled=move || busy.get() on:click=move |_| act("lock")>
                            {format!("Lock for {} hours", hours())}
                        </button>
                    }
                        .into_any()
                }
                Some(l) if l.mine => {
                    view! {
                        <button class=button disabled=move || busy.get() on:click=move |_| act("lock")>
                            "Renew"
                        </button>
                        <button class=button disabled=move || busy.get() on:click=move |_| act("unlock")>
                            "Unlock"
                        </button>
                    }
                        .into_any()
                }
                Some(_) => {
                    view! {
                        <button
                            class=move || {
                                if confirm_break.get() {
                                    "rounded bg-red-600 px-2 py-1 text-xs font-medium text-white hover:bg-red-500 disabled:opacity-50"
                                } else {
                                    button
                                }
                            }
                            title="Clear their lock, so anyone can change the ticket"
                            disabled=move || busy.get()
                            on:click=move |_| {
                                if confirm_break.get_untracked() {
                                    act("break");
                                } else {
                                    confirm_break.set(true);
                                }
                            }
                            on:blur=move |_| confirm_break.set(false)
                        >
                            {move || if confirm_break.get() { "Confirm: break their lock" } else { "Break lock" }}
                        </button>
                    }
                        .into_any()
                }
            }}
        </div>
    }
}

/// Formatting buttons for the rich editor. Each acts on mousedown and
/// cancels it, so the editor keeps its selection.
#[component]
fn Toolbar(on_change: impl Fn() + Copy + Send + Sync + 'static) -> impl IntoView {
    let link_open = RwSignal::new(false);
    let link_url = RwSignal::new(String::new());
    let saved = StoredValue::new_local(None::<web_sys::Range>);

    let button = move |label: &'static str, title: &'static str, run: fn()| {
        view! {
            <button
                type="button"
                title=title
                aria-label=title
                class="min-w-8 rounded px-2 py-1 text-sm hover:bg-slate-200 dark:hover:bg-slate-700"
                on:mousedown=move |ev| {
                    ev.prevent_default();
                    run();
                    on_change();
                }
            >
                {label}
            </button>
        }
    };

    let apply_link = move || {
        let url = link_url.get_untracked();
        if let Some(range) = saved.get_value() {
            rich_dom::restore_selection(&range);
        }
        if !url.trim().is_empty() {
            rich_dom::exec("createLink", Some(url.trim()));
            on_change();
        }
        link_open.set(false);
        link_url.set(String::new());
    };

    view! {
        <div role="toolbar" aria-label="Formatting" class="flex flex-wrap items-center gap-1">
            {button("B", "Bold", || rich_dom::exec("bold", None))}
            {button("I", "Italic", || rich_dom::exec("italic", None))}
            {button("H", "Heading", rich_dom::toggle_heading)}
            <span class="mx-1 h-5 w-px bg-slate-300 dark:bg-slate-600"></span>
            {button("•", "Bullet list", || rich_dom::exec("insertUnorderedList", None))}
            {button("1.", "Numbered list", || rich_dom::exec("insertOrderedList", None))}
            {button("☐", "Checklist item", rich_dom::toggle_checklist)}
            <span class="mx-1 h-5 w-px bg-slate-300 dark:bg-slate-600"></span>
            <button
                type="button"
                title="Link"
                aria-label="Link"
                class="min-w-8 rounded px-2 py-1 text-sm hover:bg-slate-200 dark:hover:bg-slate-700"
                on:mousedown=move |ev| {
                    ev.prevent_default();
                    saved.set_value(rich_dom::save_selection());
                    link_open.update(|o| *o = !*o);
                }
            >
                "🔗"
            </button>
            <Show when=move || link_open.get()>
                <input
                    type="url"
                    autofocus
                    placeholder="https://… then Enter"
                    aria-label="Link address"
                    class="w-64 rounded border border-slate-300 bg-slate-50 px-2 py-1 text-sm dark:border-slate-600 dark:bg-slate-900"
                    prop:value=move || link_url.get()
                    on:input=move |ev| link_url.set(event_target_value(&ev))
                    on:keydown=move |ev| {
                        if ev.key() == "Enter" {
                            ev.prevent_default();
                            apply_link();
                        }
                    }
                />
            </Show>
        </div>
    }
}
