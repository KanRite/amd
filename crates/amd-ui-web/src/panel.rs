//! The bottom panel (ticket 185), like VS Code's: on the board, a one-line
//! bar with Terminal and AI tabs that a click (or Ctrl+`) opens to a
//! terminal, draggable taller or to full height. Each tab's program starts
//! the first time it's shown and keeps running while the board is away,
//! until the project changes or amdui closes.

use amd_ui_model::terminal::{TermFrame, TermKind};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;
use crate::app::{Ctx, View};
use crate::term_view::{TermState, TermView};

/// The least an open panel can be dragged down to, in pixels.
const MIN_HEIGHT: f64 = 120.0;

/// The height from the top of the board's area (under the header and the
/// filter bar) to the bottom of the window: as tall as the panel can be.
fn room() -> f64 {
    let inner = window()
        .inner_height()
        .ok()
        .and_then(|h| h.as_f64())
        .unwrap_or(800.0);
    let top = document()
        .query_selector("main")
        .ok()
        .flatten()
        .map_or(56.0, |main| main.get_bounding_client_rect().top());
    (inner - top).max(MIN_HEIGHT)
}

#[component]
pub fn BottomPanel(project: Memo<Option<String>>) -> impl IntoView {
    let ctx = Ctx::get();
    let open = RwSignal::new(false);
    let full = RwSignal::new(false);
    let height = RwSignal::new(
        window()
            .inner_height()
            .ok()
            .and_then(|h| h.as_f64())
            .map_or(320.0, |h| (h * 0.4).max(MIN_HEIGHT)),
    );
    let tab = RwSignal::new(TermKind::Shell);
    // Bumped by the panel's buttons, so the terminal takes the keyboard back
    // rather than leaving it on the button.
    let refocus = RwSignal::new(0_u32);
    let give_back = move || refocus.update(|n| *n = n.wrapping_add(1));
    let shell = TermState::new(TermKind::Shell);
    let ai = TermState::new(TermKind::Ai);
    // On every project screen: the board, Documents and Ideas (ticket 187).
    let on_project = move || matches!(ctx.view.get(), View::Board | View::Documents | View::Ideas);
    // The editor from Preferences (ticket 188); read again each time the
    // board comes back, since Preferences is where it changes.
    let editor = RwSignal::new(false);
    Effect::new(move |_| {
        if on_project() {
            spawn_local(async move {
                if let Ok(c) = api::editor_command().await {
                    editor.set(!c.is_empty());
                }
            });
        }
    });
    // Speech to text (ticket 187): offered once a model is installed and
    // it's switched on. Read again whenever an overlay (Preferences) closes.
    let speech = RwSignal::new(None::<amd_ui_model::terminal::SpeechStatus>);
    Effect::new(move |_| {
        let closed = ctx.overlay.with(Option::is_none);
        if closed && on_project() {
            spawn_local(async move {
                if let Ok(status) = api::speech_status().await {
                    speech.set(Some(status));
                }
            });
        }
    });
    let speech_ready = move || speech.with(|s| s.as_ref().is_some_and(|s| s.ready()));
    let dictation = RwSignal::new(Dictation::Idle);
    let start_dictation = move || {
        if dictation.get_untracked() != Dictation::Idle {
            return;
        }
        if !open.get_untracked() {
            open.set(true);
        }
        dictation.set(Dictation::Recording {
            since: js_sys::Date::now(),
            toggle: false,
        });
        spawn_local(async move {
            if let Err(e) = api::speech_start().await {
                dictation.set(Dictation::Idle);
                ctx.error.set(Some(e));
            }
        });
    };
    let stop_dictation = move || {
        if !matches!(dictation.get_untracked(), Dictation::Recording { .. }) {
            return;
        }
        dictation.set(Dictation::Transcribing);
        let target = match tab.get_untracked() {
            TermKind::Shell => shell,
            TermKind::Ai => ai,
        };
        spawn_local(async move {
            match api::speech_stop().await {
                Ok(text) if !text.is_empty() => {
                    if let Some(id) = target.id.get_untracked() {
                        let _ = api::terminal_input(id, &text).await;
                    }
                }
                Ok(_) => {}
                Err(e) => ctx.error.set(Some(e)),
            }
            dictation.set(Dictation::Idle);
            give_back();
        });
    };
    // Hold Ctrl+Shift+Space to talk; letting go types what was said.
    let _talk = window_event_listener(leptos::ev::keydown, move |ev| {
        if (ev.ctrl_key() || ev.meta_key())
            && ev.shift_key()
            && ev.key() == " "
            && on_project()
            && speech_ready()
        {
            ev.prevent_default();
            if !ev.repeat() {
                start_dictation();
            }
        }
    });
    // Escape throws a recording away.
    let _cancel = window_event_listener(leptos::ev::keydown, move |ev| {
        if ev.key() == "Escape" && matches!(dictation.get_untracked(), Dictation::Recording { .. })
        {
            ev.prevent_default();
            dictation.set(Dictation::Idle);
            spawn_local(async move {
                let _ = api::speech_cancel().await;
            });
        }
    });
    let _release = window_event_listener(leptos::ev::keyup, move |ev| {
        let held = matches!(
            dictation.get_untracked(),
            Dictation::Recording { toggle: false, .. }
        );
        if held && matches!(ev.key().as_str(), " " | "Control" | "Shift" | "Meta") {
            ev.prevent_default();
            stop_dictation();
        }
    });
    // The 🎤 button: hold it to talk, or click it once to start and again
    // to stop.
    let mic_down = move |_| match dictation.get_untracked() {
        Dictation::Idle => start_dictation(),
        Dictation::Recording { toggle: true, .. } => stop_dictation(),
        _ => {}
    };
    let mic_up = move |_| {
        if let Dictation::Recording {
            since,
            toggle: false,
        } = dictation.get_untracked()
        {
            if js_sys::Date::now() - since < 350.0 {
                dictation.set(Dictation::Recording {
                    since,
                    toggle: true,
                });
            } else {
                stop_dictation();
            }
        }
    };
    let open_editor = move |_| {
        spawn_local(async move {
            if let Err(e) = api::open_in_editor().await {
                ctx.error.set(Some(e));
            }
        });
    };

    // Frames and endings from amdui, to whichever tab they belong to.
    api::on_event::<TermFrame>("terminal-frame", move |frame| {
        for state in [shell, ai] {
            if state.id.try_get_untracked().flatten() == Some(frame.id) {
                state.apply(frame);
                return;
            }
        }
    });
    api::on_event::<u32>("terminal-exit", move |id| {
        for state in [shell, ai] {
            if state.id.try_get_untracked().flatten() == Some(id) {
                state.exited.set(true);
            }
        }
    });

    // Another project: its programs end and the panel starts afresh.
    Effect::new(move |previous: Option<Option<String>>| {
        let now = project.get();
        if let Some(previous) = previous
            && previous != now
        {
            shell.close();
            ai.close();
        }
        now
    });

    // Ctrl+` opens and closes it, on the board.
    let _toggle = window_event_listener(leptos::ev::keydown, move |ev| {
        if (ev.ctrl_key() || ev.meta_key()) && ev.code() == "Backquote" && on_project() {
            ev.prevent_default();
            open.update(|o| *o = !*o);
        }
    });

    // Dragging the top edge.
    let dragging = StoredValue::new(false);
    let _drag = window_event_listener(leptos::ev::mousemove, move |ev| {
        if !dragging.get_value() {
            return;
        }
        let Some(inner) = window().inner_height().ok().and_then(|h| h.as_f64()) else {
            return;
        };
        let wanted = inner - f64::from(ev.client_y());
        height.set(wanted.clamp(MIN_HEIGHT, room()));
        full.set(false);
    });
    let _drop = window_event_listener(leptos::ev::mouseup, move |_| dragging.set_value(false));

    let size = move || {
        if !open.get() {
            String::new()
        } else if full.get() {
            format!("height:{}px;", room())
        } else {
            format!("height:{}px;", height.get().min(room()))
        }
    };
    let choose = move |kind: TermKind| {
        if open.get_untracked() && tab.get_untracked() == kind {
            open.set(false);
        } else {
            tab.set(kind);
            open.set(true);
            give_back();
        }
    };
    let tab_class = move |kind: TermKind| {
        move || {
            let base = "rounded px-2 py-0.5 text-xs";
            if open.get() && tab.get() == kind {
                format!("{base} bg-slate-700 text-white")
            } else {
                format!("{base} text-slate-300 hover:bg-slate-700/60")
            }
        }
    };
    let active = move |kind: TermKind| {
        Signal::derive(move || on_project() && open.get() && tab.get() == kind)
    };

    view! {
        <section
            aria-label="Terminal panel"
            class=move || {
                if on_project() {
                    "relative z-10 flex shrink-0 flex-col border-t border-slate-700 bg-[#1e1e1e] text-slate-200"
                } else {
                    "hidden"
                }
            }
            style=size
        >
            <Show when=move || open.get()>
                <div
                    class="h-1 shrink-0 cursor-row-resize hover:bg-accent-500"
                    title="Drag to resize"
                    on:mousedown=move |ev| {
                        ev.prevent_default();
                        dragging.set_value(true);
                    }
                ></div>
            </Show>
            <div class="flex h-8 shrink-0 items-center gap-1 px-2">
                <button type="button" class=tab_class(TermKind::Shell) on:click=move |_| choose(TermKind::Shell)>
                    "Terminal"
                </button>
                <button type="button" class=tab_class(TermKind::Ai) on:click=move |_| choose(TermKind::Ai)>
                    "AI"
                </button>
                <Show when=move || editor.get()>
                    <button
                        type="button"
                        class="rounded px-2 py-0.5 text-xs text-slate-300 hover:bg-slate-700/60"
                        title="Open the project folder in your editor"
                        on:click=open_editor
                    >
                        "Editor"
                    </button>
                </Show>
                <Show when=speech_ready>
                    <button
                        type="button"
                        class=move || match dictation.get() {
                            Dictation::Recording { .. } => "rounded bg-red-600 px-2 py-0.5 text-xs text-white",
                            Dictation::Transcribing => "rounded bg-slate-700 px-2 py-0.5 text-xs text-slate-300",
                            Dictation::Idle => "rounded px-2 py-0.5 text-xs text-slate-300 hover:bg-slate-700/60",
                        }
                        title="Hold to talk, or click to start and click again to stop (or hold Ctrl+Shift+Space). What you say is typed into the open tab."
                        aria-label="Dictate"
                        on:mousedown=mic_down
                        on:mouseup=mic_up
                    >
                        {move || match dictation.get() {
                            Dictation::Recording { .. } => "🎤 listening…",
                            Dictation::Transcribing => "🎤 transcribing…",
                            Dictation::Idle => "🎤",
                        }}
                    </button>
                </Show>
                <span class="flex-1 truncate px-2 text-xs text-slate-500">
                    {move || if open.get() { "" } else { "Ctrl+` opens the panel" }}
                </span>
                <Show when=move || open.get()>
                    <button
                        type="button"
                        class="rounded px-2 py-0.5 text-xs text-slate-300 hover:bg-slate-700/60"
                        title=move || if full.get() { "Back to the height you dragged it to" } else { "Full height" }
                        on:click=move |_| {
                            full.update(|f| *f = !*f);
                            give_back();
                        }
                    >
                        {move || if full.get() { "▾ Restore" } else { "▴ Full height" }}
                    </button>
                </Show>
                <button
                    type="button"
                    class="rounded px-2 py-0.5 text-xs text-slate-300 hover:bg-slate-700/60"
                    aria-label=move || if open.get() { "Collapse the panel" } else { "Open the panel" }
                    title="Ctrl+`"
                    on:click=move |_| open.update(|o| *o = !*o)
                >
                    {move || if open.get() { "▾" } else { "▴" }}
                </button>
            </div>
            // Both stay mounted, so a hidden tab keeps its screen.
            <div class=move || if open.get() && tab.get() == TermKind::Shell { "min-h-0 flex-1" } else { "hidden" }>
                <TermView state=shell active=active(TermKind::Shell) refocus=refocus.into() />
            </div>
            <div class=move || if open.get() && tab.get() == TermKind::Ai { "min-h-0 flex-1" } else { "hidden" }>
                <TermView state=ai active=active(TermKind::Ai) refocus=refocus.into() />
            </div>
        </section>
    }
}

/// Where dictation is (ticket 187).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Dictation {
    Idle,
    /// Since when (ms), and whether a click started it (so a second click
    /// stops it) rather than a hold.
    Recording {
        since: f64,
        toggle: bool,
    },
    Transcribing,
}
