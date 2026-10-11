//! The rich-text toolbar, shared by the ticket editor and documents
//! (ticket 176): bold, italic, heading, bullet and numbered lists,
//! checklist, link. It acts on whatever `contenteditable` area has focus.

use leptos::prelude::*;

use crate::rich_dom;

/// Formatting buttons for the rich editor. Each acts on mousedown and
/// cancels it, so the editor keeps its selection.
#[component]
pub fn Toolbar(on_change: impl Fn() + Copy + Send + Sync + 'static) -> impl IntoView {
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
