//! Documents (ticket 129): the project's documents repository and the one
//! every project shares, listed by folder on the left and read on the right.
//! Reading only: a document changes through a pull request.
//!
//! The reader shows HTML amdui renders (`amd-ui`'s `documents.rs`), in which
//! nothing runs: raw HTML is text, images are inlined. Clicks on its links
//! are caught here, so nothing navigates the window — `#doc:` links open in
//! the reader, web and mail links in the system's browser or mail program.

use amd_ui_model::{DocSource, RenderedDoc};
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
            let first = sources.with(|s| {
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
                {move || match sources.get() {
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
                        view! {
                            <div class="mx-auto max-w-4xl p-8">
                                <p class="mb-4 font-mono text-xs text-slate-500">{place}</p>
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
