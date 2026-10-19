//! The update bar (ticket 192): across the top of every screen when a newer
//! agile-md is out, in orange, a colour no accent uses. **Update** says how
//! (amdui applying it itself is ticket 191); **Later** hides that version
//! for a day, across restarts; a still newer release shows at once.

use amd_ui_model::{INSTALL_COMMAND, UpdateInfo};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;

/// How often the page asks again; amdui asks GitHub at most daily anyway.
const RECHECK: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

#[component]
pub fn UpdateBar() -> impl IntoView {
    let offer = RwSignal::new(None::<UpdateInfo>);
    let how = RwSignal::new(false);
    let ask = move || {
        spawn_local(async move {
            // Silent on failure: no network is nothing to report.
            if let Ok(found) = api::update_status().await {
                offer.set(found);
            }
        });
    };
    ask();
    let _ = set_interval_with_handle(ask, RECHECK);
    let later = move |_| {
        let Some(version) = offer.with_untracked(|o| o.as_ref().map(|o| o.version.clone())) else {
            return;
        };
        offer.set(None);
        how.set(false);
        spawn_local(async move {
            let _ = api::snooze_update(&version).await;
        });
    };
    let notes = move |_| {
        if let Some(url) = offer.with_untracked(|o| o.as_ref().map(|o| o.notes_url.clone())) {
            spawn_local(async move {
                let _ = api::open_external(&url).await;
            });
        }
    };
    let button = "rounded px-2.5 py-1 text-sm font-medium focus:outline-2 focus:outline-offset-1 focus:outline-orange-700";
    view! {
        <Show when=move || offer.with(Option::is_some)>
            <section
                role="status"
                aria-live="polite"
                aria-label="Update available"
                class="flex flex-col gap-2 border-b border-orange-300 bg-orange-100 px-4 py-2 text-orange-950 dark:border-orange-700 dark:bg-orange-900 dark:text-orange-50"
            >
                <div class="flex flex-wrap items-center gap-3">
                    <p class="flex-1 text-sm">
                        {move || {
                            offer
                                .with(|o| {
                                    o.as_ref().map(|o| {
                                        let behind = if o.behind > 1 {
                                            format!(", {} releases behind", o.behind)
                                        } else {
                                            String::new()
                                        };
                                        format!(
                                            "agile-md {} is available. You have {}{behind}.",
                                            o.version, o.current
                                        )
                                    })
                                })
                                .unwrap_or_default()
                        }}
                        " "
                        <button class="underline" on:click=notes>"What's new"</button>
                    </p>
                    <button
                        class=format!("{button} bg-orange-600 text-white hover:bg-orange-500")
                        aria-expanded=move || how.get().to_string()
                        on:click=move |_| how.update(|h| *h = !*h)
                    >
                        "Update"
                    </button>
                    <button
                        class=format!("{button} border border-orange-400 hover:bg-orange-200 dark:border-orange-500 dark:hover:bg-orange-800")
                        title="Hide this version for a day"
                        on:click=later
                    >
                        "Later"
                    </button>
                </div>
                <Show when=move || how.get()>
                    <div class="flex flex-col gap-1 text-sm">
                        <label for="update-command">
                            "Run this in a terminal, then restart amdui. Your boards, documents and settings aren't touched."
                        </label>
                        <input
                            id="update-command"
                            readonly
                            class="w-full rounded border border-orange-400 bg-white px-2 py-1 font-mono text-xs text-slate-900 dark:bg-slate-900 dark:text-slate-100"
                            prop:value=INSTALL_COMMAND
                            on:focus=move |ev| {
                                use wasm_bindgen::JsCast;
                                if let Some(input) = ev
                                    .target()
                                    .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                                {
                                    input.select();
                                }
                            }
                        />
                    </div>
                </Show>
            </section>
        </Show>
    }
}
