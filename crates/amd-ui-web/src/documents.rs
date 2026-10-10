//! Documents (ticket 129): the project's documents repository and the one
//! every project shares, listed by folder on the left and read on the right;
//! from the landing page, one folder of one of them (ticket 168).
//! Each document has a History panel (ticket 169): its versions from git,
//! any of them readable, and Restore, which puts one back as a new commit.
//!
//! The reader shows HTML amdui renders (`amd-ui`'s `documents.rs`), in which
//! nothing runs: raw HTML is text, images are inlined. Clicks on its links
//! are caught here, so nothing navigates the window — `#doc:` links open in
//! the reader, web and mail links in the system's browser or mail program.

use amd_ui_model::{DocSource, DocVersion, RenderedDoc};
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;

use crate::api;
use crate::app::Ctx;

/// The document open in the reader: its repository and path.
pub type Open = Option<(String, String)>;

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
    let restore_note = RwSignal::new(None::<String>);
    let load_history = move || {
        if let Some((source, path)) = open.get_untracked() {
            spawn_local(async move {
                history.set(Some(api::document_history(&source, &path).await));
            });
        }
    };
    // Another document: back to its current version, panel closed.
    Effect::new(move |_| {
        open.track();
        show_history.set(false);
        history.set(None);
        restore_note.set(None);
    });
    let view_version = move |commit: Option<String>| {
        let Some((source, path)) = open.get_untracked() else {
            return;
        };
        restore_note.set(None);
        spawn_local(async move {
            let read = match commit {
                Some(commit) => api::read_document_version(&source, &path, &commit).await,
                None => api::read_document(&source, &path).await,
            };
            doc.set(Some(read));
        });
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
                Err(e) => restore_note.set(Some(e)),
            }
        });
    };

    // Only the scope's repository and folder (ticket 168), or everything.
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
                            src
                        })
                        .collect(),
                })
            })
        })
    });

    let load_list = move || {
        spawn_local(async move { sources.set(Some(api::list_documents().await)) });
    };
    load_list();

    // Opens whatever `open` names. With nothing open, picks the first
    // README (or first document) and names it, so the list shows it as
    // open too; this runs again for the pick and reads it then.
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

    // Another document changed on disk (a pull, an edit): read both again.
    Effect::new(move |first: Option<()>| {
        ctx.disk_tick.track();
        if first.is_some() {
            load_list();
            if let Some((source, path)) = open.get_untracked() {
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
                        list.into_iter()
                            .map(|source| view! { <SourceTree source open /> })
                            .collect_view()
                            .into_any()
                    }
                }}
            </nav>
            <article class="min-w-0 flex-1 overflow-y-auto bg-slate-50 dark:bg-slate-900">
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
                        view! {
                            <div class="mx-auto max-w-4xl p-8">
                                <div class="mb-4 flex items-center gap-3">
                                    <p class="flex-1 font-mono text-xs text-slate-500">{place}</p>
                                    <button
                                        type="button"
                                        class="rounded border border-slate-300 px-2 py-0.5 text-xs hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
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
                                </div>
                                <Show when=move || show_history.get()>
                                    <HistoryPanel history showing=showing.clone() view_version />
                                </Show>
                                {move || {
                                    restore_note
                                        .get()
                                        .map(|e| view! { <p role="alert" class="mb-3 text-sm text-red-700 dark:text-red-300">{e}</p> })
                                }}
                                {banner}
                                <div class="doc" inner_html=d.html on:click=on_click></div>
                            </div>
                        }
                            .into_any()
                    }
                }}
            </article>
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

/// One repository's documents, grouped by folder.
#[component]
fn SourceTree(source: DocSource, open: RwSignal<Open>) -> impl IntoView {
    let heading = if source.shared {
        format!("{} (shared)", source.name)
    } else {
        source.name.clone()
    };
    let body = if !source.cloned {
        view! {
            <p class="px-2 text-xs text-slate-500">
                {format!("Not cloned on this machine. Clone {} beside the project to read it here.", source.name)}
            </p>
        }
        .into_any()
    } else if source.documents.is_empty() {
        view! { <p class="px-2 text-xs text-slate-500">"No documents yet."</p> }.into_any()
    } else {
        let name = source.name.clone();
        folders(&source)
            .into_iter()
            .map(|(folder, docs)| {
                let name = name.clone();
                view! {
                    {(!folder.is_empty())
                        .then(|| {
                            view! {
                                <p class="mt-2 px-2 text-xs font-semibold uppercase tracking-wide text-slate-500">
                                    {folder.clone()}
                                </p>
                            }
                        })}
                    <ul>
                        {docs
                            .into_iter()
                            .map(|(path, title)| {
                                let target = (name.clone(), path.clone());
                                let mine = target.clone();
                                let current = Memo::new(move |_| open.with(|o| o.as_ref() == Some(&mine)));
                                view! {
                                    <li>
                                        <button
                                            type="button"
                                            class="w-full truncate rounded px-2 py-1 text-left hover:bg-slate-100 dark:hover:bg-slate-700"
                                            class=("bg-accent-100", move || current.get())
                                            class=("dark:bg-accent-900", move || current.get())
                                            title=path.clone()
                                            on:click=move |_| open.set(Some(target.clone()))
                                        >
                                            {title}
                                        </button>
                                    </li>
                                }
                            })
                            .collect_view()}
                    </ul>
                }
            })
            .collect_view()
            .into_any()
    };
    view! {
        <section class="mb-4">
            <h2 class="mb-1 px-2 text-sm font-semibold">{heading}</h2>
            {body}
        </section>
    }
}

/// `source`'s documents by folder: the top level first, then each folder in
/// order, each document as `(path, title)`.
fn folders(source: &DocSource) -> Vec<(String, Vec<(String, String)>)> {
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut docs: Vec<_> = source.documents.iter().collect();
    docs.sort_by_key(|d| (d.path.contains('/'), d.path.clone()));
    for d in docs {
        let folder = d
            .path
            .rsplit_once('/')
            .map(|(dir, _)| dir.to_string())
            .unwrap_or_default();
        match out.last_mut() {
            Some((last, list)) if *last == folder => list.push((d.path.clone(), d.title.clone())),
            _ => out.push((folder, vec![(d.path.clone(), d.title.clone())])),
        }
    }
    out
}
