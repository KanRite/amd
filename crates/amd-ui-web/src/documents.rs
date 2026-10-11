//! Documents (ticket 129): the project's documents repository and the one
//! every project shares, listed by folder on the left and read on the right;
//! from the landing page, one folder of one of them (ticket 168).
//! Each document has a History panel (ticket 169): its versions from git,
//! any of them readable, and Restore, which puts one back as a new commit.
//!
//! Ticket 176 makes them changeable, every change committed and pushed
//! straight to main (ADR 0007): a document is read-only until **Edit**
//! (rich text, or Markdown when it holds what rich text can't keep), and
//! can be marked **Archived** (muted, with a badge), **moved** (drag it onto
//! a folder, or Move…; links to it keep working), or made new; folders can
//! be made, and removed when empty.
//!
//! The reader shows HTML amdui renders (`amd-ui`'s `documents.rs`), in which
//! nothing runs: raw HTML is text, images are inlined. Clicks on its links
//! are caught here, so nothing navigates the window — `#doc:` links open in
//! the reader, web and mail links in the system's browser or mail program.

use amd_ui_model::rich::{parse_markdown, to_html, to_markdown};
use amd_ui_model::{DocChange, DocEdit, DocSource, DocVersion, RenderedDoc};
use leptos::html::Div;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;
use web_sys::DragEvent;

use crate::api;
use crate::app::Ctx;
use crate::rich_dom;

/// The document open in the reader: its repository and path.
pub type Open = Option<(String, String)>;

/// What the sidebar is asking a name for, under a folder heading.
#[derive(Clone, PartialEq)]
struct Pending {
    source: String,
    folder: String,
    document: bool,
}

/// What the sidebar can do, handed to each repository's tree.
#[derive(Clone, Copy)]
struct Actions {
    open: RwSignal<Open>,
    /// Asked before leaving a document with unsaved edits.
    leave: RwSignal<Open>,
    dirty: RwSignal<bool>,
    dragging: RwSignal<Option<(String, String)>>,
    pending: RwSignal<Option<Pending>>,
    name: RwSignal<String>,
    new_folder: Callback<(String, String, String)>,
    delete_folder: Callback<(String, String)>,
    new_document: Callback<(String, String, String)>,
    move_document: Callback<(String, String, String)>,
}

const SMALL: &str = "rounded border border-slate-300 px-2 py-0.5 text-xs hover:bg-slate-100 disabled:opacity-40 dark:border-slate-600 dark:hover:bg-slate-700";

#[component]
pub fn DocumentsView() -> impl IntoView {
    let ctx = Ctx::get();
    let sources = RwSignal::new(None::<Result<Vec<DocSource>, String>>);
    let doc = RwSignal::new(None::<Result<RenderedDoc, String>>);
    let open = ctx.document;
    // The open document's History panel (ticket 169): its versions, whether
    // it's showing, and what a Restore said when it failed.
    let history = RwSignal::new(None::<Result<Vec<DocVersion>, String>>);
    let show_history = RwSignal::new(false);
    let note = RwSignal::new(None::<String>);
    // Editing (ticket 176): what was opened, rich or Markdown, the Markdown
    // text, the rich area, and whether anything changed.
    let editing = RwSignal::new(None::<DocEdit>);
    let rich = RwSignal::new(true);
    let markdown = RwSignal::new(String::new());
    let touched = RwSignal::new(false);
    let dirty = RwSignal::new(false);
    let editor = NodeRef::<Div>::new();
    let confirm_cancel = RwSignal::new(false);
    let leave = RwSignal::new(None::<(String, String)>);
    let moving = RwSignal::new(false);

    let load_history = move || {
        if let Some((source, path)) = open.get_untracked() {
            spawn_local(async move {
                history.set(Some(api::document_history(&source, &path).await));
            });
        }
    };
    // Another document: back to its current version, panel closed, not
    // editing.
    Effect::new(move |_| {
        open.track();
        show_history.set(false);
        history.set(None);
        note.set(None);
        editing.set(None);
        dirty.set(false);
        moving.set(false);
    });
    let view_version = move |commit: Option<String>| {
        let Some((source, path)) = open.get_untracked() else {
            return;
        };
        note.set(None);
        spawn_local(async move {
            let read = match commit {
                Some(commit) => api::read_document_version(&source, &path, &commit).await,
                None => api::read_document(&source, &path).await,
            };
            doc.set(Some(read));
        });
    };

    let load_list = move || {
        spawn_local(async move { sources.set(Some(api::list_documents().await)) });
    };
    load_list();

    // After any change: say so, read the list again, and follow the
    // document to where it is now.
    let after = move |source: String, change: DocChange| {
        ctx.report(change.sync);
        load_list();
        match change.path {
            Some(path) => open.set(Some((source, path))),
            None => {
                if let Some((source, path)) = open.get_untracked() {
                    spawn_local(async move {
                        doc.set(Some(api::read_document(&source, &path).await));
                        history.set(None);
                    });
                }
            }
        }
    };

    let restore = move |commit: String| {
        let Some((source, path)) = open.get_untracked() else {
            return;
        };
        spawn_local(async move {
            match api::restore_document(&source, &path, &commit).await {
                Ok(report) => {
                    ctx.report(report);
                    doc.set(Some(api::read_document(&source, &path).await));
                    history.set(Some(api::document_history(&source, &path).await));
                }
                Err(e) => note.set(Some(e)),
            }
        });
    };
    let archive = move |on: bool| {
        let Some((source, path)) = open.get_untracked() else {
            return;
        };
        spawn_local(async move {
            match api::archive_document(&source, &path, on).await {
                Ok(change) => after(source, change),
                Err(e) => note.set(Some(e)),
            }
        });
    };

    let new_folder = Callback::new(move |(source, parent, name): (String, String, String)| {
        spawn_local(async move {
            match api::create_document_folder(&source, &parent, &name).await {
                Ok(change) => {
                    ctx.report(change.sync);
                    load_list();
                }
                Err(e) => note.set(Some(e)),
            }
        });
    });
    let delete_folder = Callback::new(move |(source, folder): (String, String)| {
        spawn_local(async move {
            match api::delete_document_folder(&source, &folder).await {
                Ok(change) => {
                    ctx.report(change.sync);
                    load_list();
                }
                Err(e) => note.set(Some(e)),
            }
        });
    });
    let move_document = Callback::new(move |(source, path, folder): (String, String, String)| {
        spawn_local(async move {
            match api::move_document(&source, &path, &folder).await {
                Ok(change) => {
                    // Follow it only when it's the one being read.
                    let reading =
                        open.with_untracked(|o| o.as_ref().is_some_and(|(_, p)| *p == path));
                    if reading {
                        after(source, change);
                    } else {
                        ctx.report(change.sync);
                        load_list();
                    }
                }
                Err(e) => note.set(Some(e)),
            }
        });
    });

    // Rich editing: fill the area once it's there, and read it back.
    let render_rich = move || {
        if let Some(el) = editor.get_untracked() {
            el.set_inner_html(&to_html(&parse_markdown(&markdown.get_untracked())));
        }
    };
    Effect::new(move |_| {
        if editor.get().is_some() && editing.with(Option::is_some) && rich.get() {
            rich_dom::use_paragraphs();
            render_rich();
        }
    });
    let body_now = move || -> String {
        match (
            rich.get_untracked() && touched.get_untracked(),
            editor.get_untracked(),
        ) {
            (true, Some(el)) => to_markdown(&rich_dom::read_blocks(&el)),
            _ => markdown.get_untracked(),
        }
    };
    let edit = move || {
        let Some((source, path)) = open.get_untracked() else {
            return;
        };
        spawn_local(async move {
            match api::open_document_for_edit(&source, &path).await {
                Ok(e) => {
                    rich.set(e.unsupported.is_empty());
                    markdown.set(e.body.clone());
                    touched.set(false);
                    dirty.set(false);
                    confirm_cancel.set(false);
                    show_history.set(false);
                    editing.set(Some(e));
                }
                Err(e) => note.set(Some(e)),
            }
        });
    };
    let new_document = Callback::new(move |(source, folder, title): (String, String, String)| {
        spawn_local(async move {
            match api::create_document(&source, &folder, &title).await {
                Ok(change) => {
                    after(source, change);
                    // Straight into editing it.
                    spawn_local(async move {
                        // Let the open document settle before opening it
                        // for editing.
                        edit();
                    });
                }
                Err(e) => note.set(Some(e)),
            }
        });
    });
    let mark_dirty = move || {
        touched.set(true);
        let now = body_now();
        dirty.set(editing.with_untracked(|e| e.as_ref().is_some_and(|e| e.body != now)));
    };
    let toggle_mode = move || {
        if rich.get_untracked() {
            markdown.set(body_now());
            touched.set(false);
            rich.set(false);
        } else {
            touched.set(false);
            rich.set(true);
            render_rich();
        }
    };
    let save = move || {
        let Some(e) = editing.get_untracked() else {
            return;
        };
        let text = format!("{}{}", e.frontmatter, body_now());
        spawn_local(async move {
            match api::save_document(&e.source, &e.path, &text, &e.version).await {
                Ok(change) => {
                    editing.set(None);
                    dirty.set(false);
                    after(e.source.clone(), change);
                }
                Err(err) => note.set(Some(err)),
            }
        });
    };
    let cancel = move || {
        if dirty.get_untracked() && !confirm_cancel.get_untracked() {
            confirm_cancel.set(true);
            return;
        }
        editing.set(None);
        dirty.set(false);
        confirm_cancel.set(false);
    };

    // Opens whatever `open` names. With nothing open, picks the first
    // README (or first document) and names it, so the list shows it as
    // open too; this runs again for the pick and reads it then.
    let shown = Memo::new(move |_| {
        let scope = ctx.doc_scope.get();
        sources.with(|s| {
            s.clone().map(|r| {
                r.map(|list| match &scope {
                    None => list,
                    Some(scope) => list
                        .into_iter()
                        .filter(|src| src.shared == scope.shared)
                        .map(|mut src| {
                            src.documents.retain(|d| scope.holds(&d.path));
                            src.folders.retain(|f| scope.holds(f) || *f == scope.folder);
                            src
                        })
                        .collect(),
                })
            })
        })
    });
    Effect::new(move |_| match open.get() {
        Some((source, path)) => {
            spawn_local(async move { doc.set(Some(api::read_document(&source, &path).await)) });
        }
        None => {
            let first = shown.with(|s| {
                let s = s.as_ref()?.as_ref().ok()?;
                let source = s.iter().find(|s| !s.documents.is_empty())?;
                let readme = source
                    .documents
                    .iter()
                    .find(|d| d.path.eq_ignore_ascii_case("README.md"))
                    .or_else(|| source.documents.first())?;
                Some((source.name.clone(), readme.path.clone()))
            });
            if first.is_some() {
                open.set(first);
            }
        }
    });

    // Another document changed on disk (a pull, an edit): read both again,
    // unless the open one is being edited (its save checks for that).
    Effect::new(move |first: Option<()>| {
        ctx.disk_tick.track();
        if first.is_some() {
            load_list();
            if editing.with_untracked(Option::is_none)
                && let Some((source, path)) = open.get_untracked()
            {
                spawn_local(async move {
                    doc.set(Some(api::read_document(&source, &path).await));
                });
            }
        }
    });

    let on_click = move |ev: leptos::ev::MouseEvent| {
        let Some(target) = ev.target() else { return };
        let Ok(element) = target.dyn_into::<web_sys::Element>() else {
            return;
        };
        let Ok(Some(anchor)) = element.closest("a") else {
            return;
        };
        let href = anchor.get_attribute("href").unwrap_or_default();
        // A heading in this document: let the page scroll to it.
        if href.starts_with('#') && !href.starts_with("#doc:") && href.len() > 1 {
            return;
        }
        ev.prevent_default();
        if let Some(path) = href.strip_prefix("#doc:") {
            let path = js_sys::decode_uri_component(path)
                .ok()
                .and_then(|p| p.as_string())
                .unwrap_or_else(|| path.to_string());
            let source = doc.with_untracked(|d| {
                d.as_ref()
                    .and_then(|d| d.as_ref().ok())
                    .map(|d| d.source.clone())
            });
            if let Some(source) = source {
                open.set(Some((source, path)));
            }
        } else if href.starts_with("http://")
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

    let actions = Actions {
        open,
        leave,
        dirty,
        dragging: RwSignal::new(None),
        pending: RwSignal::new(None),
        name: RwSignal::new(String::new()),
        new_folder,
        delete_folder,
        new_document,
        move_document,
    };
    // The folders of the open document's repository, for Move….
    let folders_of = move |source: &str| -> Vec<String> {
        sources.with(|s| {
            s.as_ref()
                .and_then(|r| r.as_ref().ok())
                .and_then(|list| list.iter().find(|x| x.name == source))
                .map(|x| x.folders.clone())
                .unwrap_or_default()
        })
    };

    view! {
        <div class="flex h-full min-h-0">
            <nav
                aria-label="Documents"
                class="w-72 shrink-0 overflow-y-auto border-r border-slate-200 bg-white p-3 text-sm dark:border-slate-700 dark:bg-slate-800"
            >
                {move || {
                    ctx.doc_scope
                        .get()
                        .map(|scope| {
                            let what = if scope.folder.is_empty() {
                                "Shared documents".to_string()
                            } else {
                                format!("Only {}/", scope.folder)
                            };
                            view! {
                                <div class="mb-3 flex items-center justify-between gap-2 px-2 text-xs text-slate-500">
                                    <span>{what}</span>
                                    <button
                                        type="button"
                                        class="hover:underline"
                                        on:click=move |_| {
                                            ctx.doc_scope.set(None);
                                            ctx.document.set(None);
                                        }
                                    >
                                        "Show everything"
                                    </button>
                                </div>
                            }
                        })
                }}
                {move || match shown.get() {
                    None => view! { <p class="text-slate-500">"Loading…"</p> }.into_any(),
                    Some(Err(e)) => {
                        view! { <p role="alert" class="text-red-700 dark:text-red-300">{e}</p> }
                            .into_any()
                    }
                    Some(Ok(list)) => {
                        let scope = ctx.doc_scope.get().map(|s| s.folder).unwrap_or_default();
                        list.into_iter()
                            .map(|source| view! { <SourceTree source actions top=scope.clone() /> })
                            .collect_view()
                            .into_any()
                    }
                }}
            </nav>
            <article class="min-w-0 flex-1 overflow-y-auto bg-slate-50 dark:bg-slate-900">
                {move || {
                    leave
                        .get()
                        .map(|target| {
                            view! {
                                <div
                                    role="alert"
                                    class="m-4 flex flex-wrap items-center gap-3 rounded border border-amber-400 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-500 dark:bg-amber-950/40 dark:text-amber-100"
                                >
                                    <span class="flex-1">"You have unsaved changes to this document."</span>
                                    <button
                                        type="button"
                                        class="hover:underline"
                                        on:click=move |_| {
                                            dirty.set(false);
                                            leave.set(None);
                                            open.set(Some(target.clone()));
                                        }
                                    >
                                        "Discard them"
                                    </button>
                                    <button type="button" class="hover:underline" on:click=move |_| leave.set(None)>
                                        "Keep editing"
                                    </button>
                                </div>
                            }
                        })
                }}
                {move || match doc.get() {
                    None => {
                        view! {
                            <p class="p-8 text-sm text-slate-500">"Pick a document on the left."</p>
                        }
                            .into_any()
                    }
                    Some(Err(e)) => {
                        view! {
                            <p role="alert" class="p-8 text-sm text-red-700 dark:text-red-300">
                                {format!("Couldn't open that document: {e}")}
                            </p>
                        }
                            .into_any()
                    }
                    Some(Ok(d)) => {
                        let place = format!("{} / {}", d.source, d.path);
                        let old = d.version.is_some();
                        let archived = d.archived;
                        let banner = d.version.clone().map(|v| {
                            let commit = v.commit.clone();
                            view! {
                                <div
                                    role="status"
                                    class="mb-4 flex flex-wrap items-center gap-3 rounded border border-amber-400 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-500 dark:bg-amber-950/40 dark:text-amber-100"
                                >
                                    <span class="flex-1">
                                        {format!("The version of {} by {} ({}).", v.date, v.author, v.short)}
                                    </span>
                                    <button
                                        type="button"
                                        class="rounded bg-accent-600 px-3 py-1 text-sm font-medium text-white hover:bg-accent-500"
                                        on:click=move |_| restore(commit.clone())
                                    >
                                        "Restore this version"
                                    </button>
                                    <button
                                        type="button"
                                        class="hover:underline"
                                        on:click=move |_| view_version(None)
                                    >
                                        "Back to the current version"
                                    </button>
                                </div>
                            }
                        });
                        let showing = d.version.as_ref().map(|v| v.commit.clone());
                        let folder_choices = folders_of(&d.source);
                        let here = d.path.rsplit_once('/').map(|(f, _)| f.to_string()).unwrap_or_default();
                        let (move_source, move_path) = (d.source.clone(), d.path.clone());
                        view! {
                            <div class="mx-auto max-w-4xl p-8">
                                <div class="mb-4 flex flex-wrap items-center gap-2">
                                    <p class="flex-1 font-mono text-xs text-slate-500">{place}</p>
                                    <Show when=move || !old && editing.with(Option::is_none)>
                                        <button type="button" class=SMALL on:click=move |_| edit()>
                                            "Edit"
                                        </button>
                                        <button type="button" class=SMALL on:click=move |_| archive(!archived)>
                                            {if archived { "Unarchive" } else { "Archive" }}
                                        </button>
                                        <button
                                            type="button"
                                            class=SMALL
                                            aria-expanded=move || moving.get().to_string()
                                            on:click=move |_| moving.update(|m| *m = !*m)
                                        >
                                            "Move…"
                                        </button>
                                    </Show>
                                    <Show when=move || editing.with(Option::is_none)>
                                        <button
                                            type="button"
                                            class=SMALL
                                            aria-expanded=move || show_history.get().to_string()
                                            on:click=move |_| {
                                                let opening = !show_history.get_untracked();
                                                show_history.set(opening);
                                                if opening {
                                                    load_history();
                                                }
                                            }
                                        >
                                            "History"
                                        </button>
                                    </Show>
                                </div>
                                <Show when=move || moving.get()>
                                    <MovePicker
                                        folders=folder_choices.clone()
                                        here=here.clone()
                                        on_pick=Callback::new({
                                            let (source, path) = (move_source.clone(), move_path.clone());
                                            move |folder: String| {
                                                moving.set(false);
                                                move_document.run((source.clone(), path.clone(), folder));
                                            }
                                        })
                                    />
                                </Show>
                                <Show when=move || show_history.get()>
                                    <HistoryPanel history showing=showing.clone() view_version />
                                </Show>
                                {move || {
                                    note.get()
                                        .map(|e| view! { <p role="alert" class="mb-3 text-sm text-red-700 dark:text-red-300">{e}</p> })
                                }}
                                {banner}
                                {(archived && !old)
                                    .then(|| {
                                        view! {
                                            <div
                                                role="status"
                                                class="mb-4 flex items-center gap-3 rounded border border-slate-300 bg-slate-100 px-3 py-2 text-sm text-slate-600 dark:border-slate-600 dark:bg-slate-800 dark:text-slate-300"
                                            >
                                                <span class="flex-1">"Archived: kept for the record, no longer current."</span>
                                                <button type="button" class="hover:underline" on:click=move |_| archive(false)>
                                                    "Unarchive"
                                                </button>
                                            </div>
                                        }
                                    })}
                                {move || match editing.get() {
                                    None => {
                                        view! {
                                            <div
                                                class="doc"
                                                class=("opacity-70", archived)
                                                inner_html=d.html.clone()
                                                on:click=on_click
                                            ></div>
                                        }
                                            .into_any()
                                    }
                                    Some(e) => {
                                        let locked = !e.unsupported.is_empty();
                                        let why = e.unsupported.join(", ");
                                        view! {
                                            <div class="flex flex-col gap-2">
                                                {locked
                                                    .then(|| {
                                                        view! {
                                                            <p class="text-xs text-slate-500">
                                                                {format!("This document has {why}, which rich text can't keep, so it's edited as Markdown.")}
                                                            </p>
                                                        }
                                                    })}
                                                <div class=move || if rich.get() { "flex flex-col gap-2" } else { "hidden" }>
                                                    <crate::rich_editor::Toolbar on_change=mark_dirty />
                                                    <div
                                                        node_ref=editor
                                                        contenteditable="true"
                                                        role="textbox"
                                                        aria-multiline="true"
                                                        aria-label="Document text"
                                                        class="rich-editor doc min-h-[50vh] rounded border border-slate-300 bg-white p-4 focus:outline-2 focus:outline-accent-500 dark:border-slate-600 dark:bg-slate-900"
                                                        on:input=move |_| mark_dirty()
                                                    ></div>
                                                </div>
                                                <Show when=move || !rich.get()>
                                                    <textarea
                                                        aria-label="Document Markdown"
                                                        spellcheck="false"
                                                        class="min-h-[50vh] rounded border border-slate-300 bg-white p-3 font-mono text-sm dark:border-slate-600 dark:bg-slate-900"
                                                        prop:value=move || markdown.get()
                                                        on:input=move |ev| {
                                                            markdown.set(event_target_value(&ev));
                                                            mark_dirty();
                                                        }
                                                    ></textarea>
                                                </Show>
                                                <div class="flex items-center gap-2">
                                                    <label
                                                        class="flex items-center gap-2 text-sm"
                                                        class=("opacity-40", locked)
                                                    >
                                                        <input
                                                            type="checkbox"
                                                            disabled=locked
                                                            prop:checked=move || !rich.get()
                                                            on:change=move |_| toggle_mode()
                                                        />
                                                        "Markdown"
                                                    </label>
                                                    <span class="flex-1"></span>
                                                    <button type="button" class=SMALL on:click=move |_| cancel()>
                                                        {move || if confirm_cancel.get() { "Discard changes?" } else { "Cancel" }}
                                                    </button>
                                                    <button
                                                        type="button"
                                                        class="rounded bg-accent-600 px-3 py-1 text-sm font-medium text-white hover:bg-accent-500 disabled:opacity-50"
                                                        disabled=move || !dirty.get()
                                                        on:click=move |_| save()
                                                    >
                                                        "Save"
                                                    </button>
                                                </div>
                                            </div>
                                        }
                                            .into_any()
                                    }
                                }}
                            </div>
                        }
                            .into_any()
                    }
                }}
            </article>
        </div>
    }
}

/// Where to move the open document: its repository's folders, the top
/// among them, the one it's in left out.
#[component]
fn MovePicker(folders: Vec<String>, here: String, on_pick: Callback<String>) -> impl IntoView {
    let mut choices: Vec<String> = vec![String::new()];
    choices.extend(folders);
    choices.retain(|f| *f != here);
    view! {
        <div class="mb-4 flex flex-wrap items-center gap-2 rounded border border-slate-200 bg-white p-2 text-sm dark:border-slate-700 dark:bg-slate-800">
            <span class="text-xs text-slate-500">"Move to"</span>
            <select
                aria-label="Folder to move the document to"
                class="appearance-none rounded border border-slate-300 bg-slate-50 px-2 py-1 text-sm dark:border-slate-600 dark:bg-slate-900"
                on:change=move |ev| {
                    let value = event_target_value(&ev);
                    if value != "\u{0}" {
                        on_pick.run(value);
                    }
                }
            >
                <option value="\u{0}" selected=true>"choose a folder…"</option>
                {choices
                    .into_iter()
                    .map(|f| {
                        let label = if f.is_empty() { "(top folder)".to_string() } else { format!("{f}/") };
                        view! { <option value=f>{label}</option> }
                    })
                    .collect_view()}
            </select>
        </div>
    }
}

/// A document's versions, newest first (ticket 169): the first is the
/// current one; any other opens read-only in the reader, with Restore.
#[component]
fn HistoryPanel(
    history: RwSignal<Option<Result<Vec<DocVersion>, String>>>,
    showing: Option<String>,
    view_version: impl Fn(Option<String>) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <section
            aria-label="History"
            class="mb-4 rounded border border-slate-200 bg-white p-2 text-sm dark:border-slate-700 dark:bg-slate-800"
        >
            {move || match history.get() {
                None => view! { <p class="px-2 text-xs text-slate-500">"Reading the history…"</p> }.into_any(),
                Some(Err(e)) => view! { <p role="alert" class="px-2 text-xs text-red-700 dark:text-red-300">{e}</p> }.into_any(),
                Some(Ok(list)) => {
                    let showing = showing.clone();
                    view! {
                        <ul class="flex flex-col">
                            {list
                                .into_iter()
                                .enumerate()
                                .map(|(i, v)| {
                                    let current = i == 0;
                                    let here = match &showing {
                                        Some(c) => *c == v.commit,
                                        None => current,
                                    };
                                    let commit = v.commit.clone();
                                    view! {
                                        <li>
                                            <button
                                                type="button"
                                                class="flex w-full items-baseline gap-3 rounded px-2 py-1 text-left hover:bg-slate-100 dark:hover:bg-slate-700"
                                                class=("bg-accent-100", here)
                                                class=("dark:bg-accent-900", here)
                                                title=format!("{} ({})", v.subject, v.short)
                                                on:click=move |_| {
                                                    view_version(if current { None } else { Some(commit.clone()) })
                                                }
                                            >
                                                <span class="font-mono text-xs text-slate-500">{v.date.clone()}</span>
                                                <span class="text-xs">{v.author.clone()}</span>
                                                <span class="flex-1 truncate">{v.subject.clone()}</span>
                                                {current.then(|| view! { <span class="text-xs text-slate-500">"current"</span> })}
                                            </button>
                                        </li>
                                    }
                                })
                                .collect_view()}
                        </ul>
                    }
                        .into_any()
                }
            }}
        </section>
    }
}

/// One repository's documents, folder by folder (empty folders too), each
/// folder a drop target with New document, New folder and, when empty,
/// Delete. `top` is the folder the view is scoped to ("" for the root).
#[component]
fn SourceTree(source: DocSource, actions: Actions, top: String) -> impl IntoView {
    let heading = if source.shared {
        format!("{} (shared)", source.name)
    } else {
        source.name.clone()
    };
    if !source.cloned {
        return view! {
            <section class="mb-4">
                <h2 class="mb-1 px-2 text-sm font-semibold">{heading}</h2>
                <p class="px-2 text-xs text-slate-500">
                    {format!("Not cloned on this machine. Clone {} beside the project to read it here.", source.name)}
                </p>
            </section>
        }
        .into_any();
    }
    let name = source.name.clone();
    // The scope's own folder (or the root) first, then each folder inside.
    let mut groups: Vec<String> = vec![top.clone()];
    groups.extend(source.folders.iter().filter(|f| **f != top).cloned());
    let empty = |folder: &str| {
        let prefix = format!("{folder}/");
        !source.documents.iter().any(|d| d.path.starts_with(&prefix))
    };
    let rows = groups
        .into_iter()
        .map(|folder| {
            let docs: Vec<(String, String, bool)> = source
                .documents
                .iter()
                .filter(|d| d.path.rsplit_once('/').map_or("", |(f, _)| f) == folder)
                .map(|d| (d.path.clone(), d.title.clone(), d.archived))
                .collect();
            let is_top = folder == top;
            let depth = if is_top {
                0
            } else {
                folder.matches('/').count() + 1
                    - top.matches('/').count()
                    - usize::from(!top.is_empty())
            };
            let label = if is_top {
                heading.clone()
            } else {
                folder.rsplit('/').next().unwrap_or(&folder).to_string()
            };
            let can_delete = !is_top && empty(&folder);
            view! {
                <FolderRow
                    source=name.clone()
                    folder=folder.clone()
                    label
                    depth
                    is_top
                    can_delete
                    actions
                />
                <ul style=format!("padding-left: {}rem", depth as f32 * 0.75)>
                    {docs
                        .into_iter()
                        .map(|(path, title, archived)| {
                            view! { <DocRow source=name.clone() path title archived actions /> }
                        })
                        .collect_view()}
                </ul>
            }
        })
        .collect_view();
    view! { <section class="mb-4">{rows}</section> }.into_any()
}

/// A folder's heading: a drop target for documents, with its actions, and
/// the name field when one is being made here.
#[component]
fn FolderRow(
    source: String,
    folder: String,
    label: String,
    depth: usize,
    is_top: bool,
    can_delete: bool,
    actions: Actions,
) -> impl IntoView {
    let over = RwSignal::new(false);
    let here = Pending {
        source: source.clone(),
        folder: folder.clone(),
        document: false,
    };
    let asking = {
        let here = here.clone();
        move || {
            actions.pending.with(|p| {
                p.as_ref()
                    .is_some_and(|p| p.source == here.source && p.folder == here.folder)
            })
        }
    };
    let ask = {
        let here = here.clone();
        move |document: bool| {
            actions.name.set(String::new());
            actions.pending.set(Some(Pending {
                document,
                ..here.clone()
            }));
        }
    };
    let confirm = {
        let (source, folder) = (source.clone(), folder.clone());
        move || {
            let name = actions.name.get_untracked().trim().to_string();
            let Some(p) = actions.pending.get_untracked() else {
                return;
            };
            actions.pending.set(None);
            if name.is_empty() {
                return;
            }
            if p.document {
                actions
                    .new_document
                    .run((source.clone(), folder.clone(), name));
            } else {
                actions
                    .new_folder
                    .run((source.clone(), folder.clone(), name));
            }
        }
    };
    let (drop_source, drop_folder) = (source.clone(), folder.clone());
    let (del_source, del_folder) = (source.clone(), folder.clone());
    let title_class = if is_top {
        "flex-1 truncate text-sm font-semibold"
    } else {
        "flex-1 truncate text-xs font-semibold uppercase tracking-wide text-slate-500"
    };
    view! {
        <div
            class="group mt-2 flex items-center gap-1 rounded px-2 py-0.5"
            class=("bg-accent-100", move || over.get())
            class=("dark:bg-accent-900", move || over.get())
            style=format!("margin-left: {}rem", depth as f32 * 0.75)
            on:dragover=move |ev: DragEvent| {
                let ours = actions.dragging.with_untracked(|d| d.as_ref().is_some_and(|(s, _)| *s == drop_source));
                if ours {
                    ev.prevent_default();
                    over.set(true);
                }
            }
            on:dragleave=move |_| over.set(false)
            on:drop=move |ev: DragEvent| {
                ev.prevent_default();
                over.set(false);
                if let Some((source, path)) = actions.dragging.get_untracked() {
                    actions.dragging.set(None);
                    actions.move_document.run((source, path, drop_folder.clone()));
                }
            }
        >
            <span class=title_class title=if folder.is_empty() { "(top folder)".to_string() } else { format!("{folder}/") }>
                {label}
            </span>
            <button type="button" class="text-xs text-slate-500 hover:text-slate-900 dark:hover:text-slate-100" title="New document here" on:click={
                let ask = ask.clone();
                move |_| ask(true)
            }>
                "＋doc"
            </button>
            <button type="button" class="text-xs text-slate-500 hover:text-slate-900 dark:hover:text-slate-100" title="New folder here" on:click=move |_| ask(false)>
                "＋folder"
            </button>
            {can_delete
                .then(|| {
                    view! {
                        <button
                            type="button"
                            class="text-xs text-slate-500 hover:text-red-700 dark:hover:text-red-300"
                            title="Delete this empty folder"
                            on:click=move |_| actions.delete_folder.run((del_source.clone(), del_folder.clone()))
                        >
                            "Delete"
                        </button>
                    }
                })}
        </div>
        <Show when=asking.clone()>
            <input
                type="text"
                autofocus
                class="ml-2 mt-1 w-[90%] rounded border border-slate-300 bg-slate-50 px-2 py-1 text-xs dark:border-slate-600 dark:bg-slate-900"
                placeholder=move || {
                    if actions.pending.with(|p| p.as_ref().is_some_and(|p| p.document)) {
                        "New document's title, then Enter"
                    } else {
                        "New folder's name, then Enter"
                    }
                }
                aria-label="Name"
                prop:value=move || actions.name.get()
                on:input=move |ev| actions.name.set(event_target_value(&ev))
                on:keydown={
                    let confirm = confirm.clone();
                    move |ev| match ev.key().as_str() {
                        "Enter" => {
                            ev.prevent_default();
                            confirm();
                        }
                        "Escape" => {
                            ev.stop_propagation();
                            actions.pending.set(None);
                        }
                        _ => {}
                    }
                }
            />
        </Show>
    }
}

/// A document in the list: opens it, drags to a folder, muted with a badge
/// when archived.
#[component]
fn DocRow(
    source: String,
    path: String,
    title: String,
    archived: bool,
    actions: Actions,
) -> impl IntoView {
    let target = (source.clone(), path.clone());
    let mine = target.clone();
    let current = Memo::new(move |_| actions.open.with(|o| o.as_ref() == Some(&mine)));
    let drag = target.clone();
    view! {
        <li>
            <button
                type="button"
                draggable="true"
                class="flex w-full items-center gap-2 truncate rounded px-2 py-1 text-left hover:bg-slate-100 dark:hover:bg-slate-700"
                class=("bg-accent-100", move || current.get())
                class=("dark:bg-accent-900", move || current.get())
                class=("italic", archived)
                class=("text-slate-400", archived)
                title=path.clone()
                on:dragstart=move |ev: DragEvent| {
                    if let Some(dt) = ev.data_transfer() {
                        let _ = dt.set_data("text/plain", &drag.1);
                        dt.set_effect_allowed("move");
                    }
                    actions.dragging.set(Some(drag.clone()));
                }
                on:dragend=move |_| actions.dragging.set(None)
                on:click=move |_| {
                    if actions.dirty.get_untracked() {
                        actions.leave.set(Some(target.clone()));
                    } else {
                        actions.open.set(Some(target.clone()));
                    }
                }
            >
                <span class="flex-1 truncate">{title}</span>
                {archived
                    .then(|| {
                        view! {
                            <span class="rounded bg-slate-200 px-1 text-[10px] font-semibold uppercase not-italic text-slate-600 dark:bg-slate-700 dark:text-slate-300">
                                "Archived"
                            </span>
                        }
                    })}
            </button>
        </li>
    }
}
