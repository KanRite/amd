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
    let on_board = move || ctx.view.get() == View::Board;
    // The editor from Preferences (ticket 188); read again each time the
    // board comes back, since Preferences is where it changes.
    let editor = RwSignal::new(false);
    Effect::new(move |_| {
        if on_board() {
            spawn_local(async move {
                if let Ok(c) = api::editor_command().await {
                    editor.set(!c.is_empty());
                }
            });
        }
    });
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
        if (ev.ctrl_key() || ev.meta_key()) && ev.code() == "Backquote" && on_board() {
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
    let active =
        move |kind: TermKind| Signal::derive(move || on_board() && open.get() && tab.get() == kind);

    view! {
        <section
            aria-label="Terminal panel"
            class=move || {
                if on_board() {
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
