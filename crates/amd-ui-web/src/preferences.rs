//! Preferences (ticket 148): settings that belong to you on this machine,
//! not to the project. Saved in agile-md's own config
//! (`~/.config/agile-md/config`), never committed: the colour scheme, the
//! zoom, and which board this machine opens when started from anywhere.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::app::{Ctx, Modal, ModalClose, zoom_to};

#[component]
pub fn PreferencesPanel() -> impl IntoView {
    view! {
        <Modal title="Preferences".to_string() width="max-w-2xl">
            <div class="flex min-h-0 flex-col gap-4 overflow-y-auto">
                <AppearanceSection />
                <ZoomSection />
                <BoardSection />
                <AiSection />
                <EditorSection />
            </div>
            <ModalClose />
        </Modal>
    }
}

#[component]
fn AppearanceSection() -> impl IntoView {
    use amd_ui_model::appearance::{Accent, Appearance, Theme};
    let ctx = Ctx::get();
    let problem = RwSignal::new(None::<String>);
    let choose = move |change: Appearance| {
        ctx.appearance.set(change);
        spawn_local(async move {
            match api::set_appearance(change).await {
                Ok(_) => problem.set(None),
                Err(e) => problem.set(Some(e)),
            }
        });
    };
    // Written out in full so Tailwind generates them.
    let swatch = |accent: Accent| match accent {
        Accent::Indigo => "bg-indigo-600",
        Accent::Teal => "bg-teal-600",
        Accent::Rose => "bg-rose-600",
        Accent::Amber => "bg-amber-600",
        Accent::Slate => "bg-slate-600",
    };
    view! {
        <section class="flex flex-col gap-2">
            <h3 class="text-sm font-semibold">"Appearance"</h3>
            <div role="radiogroup" aria-label="Theme" class="flex flex-wrap gap-2">
                {Theme::ALL
                    .map(|theme| {
                        let on = move || ctx.appearance.with(|a| a.theme == theme);
                        view! {
                            <button
                                type="button"
                                role="radio"
                                aria-checked=move || on().to_string()
                                class=move || {
                                    if on() {
                                        "rounded bg-accent-600 px-3 py-1.5 text-sm font-medium text-white"
                                    } else {
                                        "rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-700"
                                    }
                                }
                                on:click=move |_| {
                                    choose(Appearance { theme, ..ctx.appearance.get_untracked() })
                                }
                            >
                                {theme.label()}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
            <div role="radiogroup" aria-label="Accent colour" class="flex flex-wrap items-center gap-3">
                <span class="text-xs text-slate-500">"Accent"</span>
                {Accent::ALL
                    .map(|accent| {
                        let on = move || ctx.appearance.with(|a| a.accent == accent);
                        view! {
                            <button
                                type="button"
                                role="radio"
                                aria-checked=move || on().to_string()
                                aria-label=accent.label()
                                title=accent.label()
                                class=move || {
                                    format!(
                                        "h-7 w-7 rounded-full {} {}",
                                        swatch(accent),
                                        if on() {
                                            "ring-2 ring-offset-2 ring-slate-900 ring-offset-white dark:ring-white dark:ring-offset-slate-800"
                                        } else {
                                            "opacity-80 hover:opacity-100"
                                        },
                                    )
                                }
                                on:click=move |_| {
                                    choose(Appearance { accent, ..ctx.appearance.get_untracked() })
                                }
                            ></button>
                        }
                    })
                    .collect_view()}
            </div>
            <p class="text-xs text-slate-500">
                "Saved in your own settings on this machine, and used again next time."
            </p>
            {move || {
                problem
                    .get()
                    .map(|p| {
                        view! {
                            <p role="alert" class="text-xs font-medium text-red-700 dark:text-red-300">
                                {p}
                            </p>
                        }
                    })
            }}
        </section>
    }
}

/// The board's zoom (ticket 024): the same steps as Ctrl + and Ctrl -.
#[component]
fn ZoomSection() -> impl IntoView {
    use amd_ui_model::zoom;
    let ctx = Ctx::get();
    // Worked out here, not in the markup: `view!` would read a `>` there as
    // the end of the tag.
    let smallest = move || ctx.zoom.get() <= zoom::STEPS[0];
    let largest = move || ctx.zoom.get() >= zoom::STEPS[zoom::STEPS.len() - 1];
    let button = "rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700";
    view! {
        <section class="flex flex-col gap-2">
            <h3 class="text-sm font-semibold">"Zoom"</h3>
            <div class="flex items-center gap-2">
                <button
                    class=button
                    aria-label="Zoom out"
                    disabled=smallest
                    on:click=move |_| zoom_to(ctx, zoom::step(ctx.zoom.get_untracked(), false))
                >
                    "−"
                </button>
                <span class="w-14 text-center font-mono text-sm" aria-live="polite">
                    {move || format!("{}%", ctx.zoom.get())}
                </span>
                <button
                    class=button
                    aria-label="Zoom in"
                    disabled=largest
                    on:click=move |_| zoom_to(ctx, zoom::step(ctx.zoom.get_untracked(), true))
                >
                    "+"
                </button>
                <button
                    class=button
                    disabled=move || ctx.zoom.get() == zoom::DEFAULT
                    on:click=move |_| zoom_to(ctx, zoom::DEFAULT)
                >
                    "Reset"
                </button>
            </div>
            <p class="text-xs text-slate-500">
                "Also Ctrl + and Ctrl −, and Ctrl 0 to reset. Remembered on this machine."
            </p>
        </section>
    }
}

/// Which board this machine opens when amdui starts from outside any
/// project (a desktop launcher starts in your home directory).
#[component]
fn BoardSection() -> impl IntoView {
    let ctx = Ctx::get();
    let dir = RwSignal::new(String::new());
    let status = RwSignal::new(None::<String>);
    spawn_local(async move {
        if let Ok(Some(d)) = api::tasks_dir().await {
            dir.set(d);
        }
    });
    let open_dir = move |_| {
        let path = dir.get_untracked();
        spawn_local(async move {
            match api::set_tasks_dir(&path).await {
                Ok(board) => {
                    dir.set(board.tasks_dir.clone());
                    ctx.board.set(Some(board));
                    ctx.error.set(None);
                    status.set(Some("Board opened.".into()));
                }
                Err(e) => status.set(Some(e)),
            }
        });
    };
    let field = "rounded border border-slate-300 bg-slate-50 px-2 py-1.5 dark:border-slate-600 dark:bg-slate-900";
    let secondary = "rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700";
    view! {
        <section class="flex flex-col gap-2">
            <h3 class="text-sm font-semibold">"Board to open"</h3>
            <div class="flex gap-2">
                <input
                    type="text"
                    aria-label="Tasks directory"
                    placeholder="/path/to/project-tickets/tasks"
                    class=format!("{field} flex-1 font-mono text-sm")
                    prop:value=move || dir.get()
                    on:input=move |ev| dir.set(event_target_value(&ev))
                />
                <button class=secondary on:click=open_dir>
                    "Open"
                </button>
            </div>
            <p class="text-xs text-slate-500">
                "A directory holding backlog/, todo/, doing/ and done/. Opened when amdui starts outside a project, and remembered for next time."
            </p>
            {move || status.get().map(|s| view! { <p class="text-xs">{s}</p> })}
        </section>
    }
}

/// The command the bottom panel's AI tab runs (ticket 185): `claude` unless
/// set here. A tab already running keeps its command until it's started
/// again.
#[component]
fn AiSection() -> impl IntoView {
    let command = RwSignal::new(String::new());
    let status = RwSignal::new(None::<String>);
    spawn_local(async move {
        if let Ok(c) = api::ai_command().await {
            command.set(c);
        }
    });
    let save = move |_| {
        let value = command.get_untracked();
        spawn_local(async move {
            match api::set_ai_command(&value).await {
                Ok(saved) => {
                    command.set(saved.clone());
                    status.set(Some(format!(
                        "The AI tab runs {saved} the next time it starts."
                    )));
                }
                Err(e) => status.set(Some(e)),
            }
        });
    };
    let field = "rounded border border-slate-300 bg-slate-50 px-2 py-1.5 dark:border-slate-600 dark:bg-slate-900";
    let secondary = "rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700";
    view! {
        <section class="flex flex-col gap-2">
            <h3 class="text-sm font-semibold">"AI command"</h3>
            <div class="flex gap-2">
                <input
                    type="text"
                    aria-label="AI command"
                    placeholder="claude"
                    class=format!("{field} flex-1 font-mono text-sm")
                    prop:value=move || command.get()
                    on:input=move |ev| command.set(event_target_value(&ev))
                />
                <button class=secondary on:click=save>
                    "Save"
                </button>
            </div>
            <p class="text-xs text-slate-500">
                "What the board's AI tab runs, in the project folder, through your shell (so it's found as in a terminal). Leave it empty for claude."
            </p>
            {move || status.get().map(|s| view! { <p class="text-xs">{s}</p> })}
        </section>
    }
}

/// The editor the board's Editor button opens the project folder in (ticket
/// 188). Empty means no button.
#[component]
fn EditorSection() -> impl IntoView {
    let command = RwSignal::new(String::new());
    let status = RwSignal::new(None::<String>);
    spawn_local(async move {
        if let Ok(c) = api::editor_command().await {
            command.set(c);
        }
    });
    let save = move |_| {
        let value = command.get_untracked();
        spawn_local(async move {
            match api::set_editor_command(&value).await {
                Ok(saved) => {
                    command.set(saved.clone());
                    status.set(Some(if saved.is_empty() {
                        "No editor: the board has no Editor button.".to_string()
                    } else {
                        format!("The board's Editor button opens the project in {saved}.")
                    }));
                }
                Err(e) => status.set(Some(e)),
            }
        });
    };
    let field = "rounded border border-slate-300 bg-slate-50 px-2 py-1.5 dark:border-slate-600 dark:bg-slate-900";
    let secondary = "rounded border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 disabled:opacity-50 dark:border-slate-600 dark:hover:bg-slate-700";
    view! {
        <section class="flex flex-col gap-2">
            <h3 class="text-sm font-semibold">"Editor command"</h3>
            <div class="flex gap-2">
                <input
                    type="text"
                    aria-label="Editor command"
                    placeholder="code"
                    class=format!("{field} flex-1 font-mono text-sm")
                    prop:value=move || command.get()
                    on:input=move |ev| command.set(event_target_value(&ev))
                />
                <button class=secondary on:click=save>
                    "Save"
                </button>
            </div>
            <p class="text-xs text-slate-500">
                "The command that opens a folder in your editor, such as code, zed or idea. The board then shows an Editor button after AI, which opens the project folder in it. Empty: no button."
            </p>
            {move || status.get().map(|s| view! { <p class="text-xs">{s}</p> })}
        </section>
    }
}
