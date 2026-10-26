//! One terminal in the bottom panel (ticket 185), drawn from the frames
//! riteui sends (its screen is `alacritty_terminal`'s, in the backend). Each
//! screen row is its own signal, so a frame redraws only the rows that
//! changed. Keys go through a hidden textarea, which also takes pastes; the
//! screen is ordinary text, so selecting and copying just work.

use leptos::html::{Div, Span, Textarea};
use leptos::prelude::*;
use leptos::task::spawn_local;
use rite_ui_model::terminal::{
    TermCursor, TermFrame, TermKind, TermRun, grid_size, key_bytes, paste_bytes,
};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::Closure;

use crate::api;

/// Padding inside the terminal, in pixels: `px-2 py-1`.
const PAD_X: f64 = 8.0;
const PAD_Y: f64 = 4.0;

/// What the page knows of one terminal: its id once started, its rows, the
/// cursor and the modes that change how keys are sent.
#[derive(Clone, Copy)]
pub struct TermState {
    pub kind: TermKind,
    pub id: RwSignal<Option<u32>>,
    rows: RwSignal<Vec<ArcRwSignal<Vec<TermRun>>>>,
    cursor: RwSignal<TermCursor>,
    /// `(app_cursor, bracketed_paste, alt_screen)`.
    modes: RwSignal<(bool, bool, bool)>,
    pub exited: RwSignal<bool>,
    starting: RwSignal<bool>,
    size: RwSignal<(u16, u16)>,
}

impl TermState {
    pub fn new(kind: TermKind) -> TermState {
        TermState {
            kind,
            id: RwSignal::new(None),
            rows: RwSignal::new(Vec::new()),
            cursor: RwSignal::new(TermCursor::default()),
            modes: RwSignal::new((false, false, false)),
            exited: RwSignal::new(false),
            starting: RwSignal::new(false),
            size: RwSignal::new((0, 0)),
        }
    }

    /// A frame from riteui: the rows it carries, the cursor, the modes.
    pub fn apply(&self, frame: TermFrame) {
        let lines = frame.lines as usize;
        if self.rows.with_untracked(Vec::len) != lines {
            self.rows
                .set((0..lines).map(|_| ArcRwSignal::new(Vec::new())).collect());
        }
        self.rows.with_untracked(|rows| {
            if frame.full {
                let mut seen = vec![false; rows.len()];
                for row in &frame.rows {
                    if let Some(slot) = seen.get_mut(row.y as usize) {
                        *slot = true;
                    }
                }
                for (y, row) in rows.iter().enumerate() {
                    if !seen[y] {
                        row.set(Vec::new());
                    }
                }
            }
            for row in frame.rows {
                if let Some(signal) = rows.get(row.y as usize) {
                    signal.set(row.runs);
                }
            }
        });
        self.cursor.set(frame.cursor);
        self.modes
            .set((frame.app_cursor, frame.bracketed_paste, frame.alt_screen));
    }

    /// Back to nothing: the program has gone, or the project changed.
    pub fn reset(&self) {
        self.id.set(None);
        self.rows.set(Vec::new());
        self.exited.set(false);
        self.starting.set(false);
    }

    /// Ends the program, if there is one, and forgets it.
    pub fn close(&self) {
        if let Some(id) = self.id.get_untracked() {
            spawn_local(async move {
                let _ = api::terminal_close(id).await;
            });
        }
        self.reset();
    }
}

/// Sends input to a terminal in order: invokes can overtake each other, so
/// keystrokes queue and go one call at a time.
#[derive(Clone, Copy)]
struct Sender {
    pending: StoredValue<String>,
    busy: StoredValue<bool>,
}

impl Sender {
    fn new() -> Sender {
        Sender {
            pending: StoredValue::new(String::new()),
            busy: StoredValue::new(false),
        }
    }

    fn send(self, id: u32, text: String) {
        self.pending.update_value(|p| p.push_str(&text));
        if self.busy.get_value() {
            return;
        }
        self.busy.set_value(true);
        spawn_local(async move {
            loop {
                let next = std::mem::take(&mut *self.pending.write_value());
                if next.is_empty() {
                    break;
                }
                let _ = api::terminal_input(id, &next).await;
            }
            self.busy.set_value(false);
        });
    }
}

/// How a run looks, as inline CSS.
fn run_style(run: &TermRun) -> String {
    let mut style = String::new();
    if !run.fg.is_empty() {
        style.push_str(&format!("color:{};", run.fg));
    }
    if !run.bg.is_empty() {
        style.push_str(&format!("background:{};", run.bg));
    }
    if run.bold {
        style.push_str("font-weight:700;");
    }
    if run.italic {
        style.push_str("font-style:italic;");
    }
    match (run.underline, run.strike) {
        (true, true) => style.push_str("text-decoration:underline line-through;"),
        (true, false) => style.push_str("text-decoration:underline;"),
        (false, true) => style.push_str("text-decoration:line-through;"),
        _ => {}
    }
    if run.dim {
        style.push_str("opacity:0.6;");
    }
    style
}

/// A terminal: started the first time `active` is true, sized to its box.
#[component]
pub fn TermView(state: TermState, active: Signal<bool>, refocus: Signal<u32>) -> impl IntoView {
    let screen = NodeRef::<Div>::new();
    let measure = NodeRef::<Span>::new();
    let input = NodeRef::<Textarea>::new();
    // One character cell, in pixels, measured from the font in use.
    let cell = RwSignal::new((0.0_f64, 0.0_f64));
    let sender = Sender::new();
    let focused = RwSignal::new(false);

    let measure_cell = move || {
        if let Some(span) = measure.get_untracked() {
            let rect = span.get_bounding_client_rect();
            if rect.width() > 0.0 {
                cell.set((rect.width() / 10.0, rect.height()));
            }
        }
    };
    // The grid that fits the box now; `None` while hidden or not measured.
    let fit = move || -> Option<(u16, u16)> {
        let (cw, ch) = cell.get_untracked();
        let el = screen.get_untracked()?;
        let rect = el.get_bounding_client_rect();
        if cw <= 0.0 || rect.width() < 40.0 || rect.height() < ch {
            return None;
        }
        Some(grid_size(
            rect.width() - 2.0 * PAD_X,
            rect.height() - 2.0 * PAD_Y,
            cw,
            ch,
        ))
    };
    let start = move || {
        if state.id.get_untracked().is_some() || state.starting.get_untracked() {
            return;
        }
        measure_cell();
        let Some((cols, rows)) = fit() else { return };
        state.starting.set(true);
        state.exited.set(false);
        state.size.set((cols, rows));
        spawn_local(async move {
            match api::terminal_open(state.kind, cols, rows).await {
                Ok(id) => state.id.set(Some(id)),
                Err(e) => state.rows.set(vec![ArcRwSignal::new(vec![TermRun {
                    text: e,
                    fg: "#f14c4c".into(),
                    ..TermRun::default()
                }])]),
            }
            state.starting.set(false);
        });
    };
    let focus = move || {
        if let Some(el) = input.get_untracked() {
            let _ = el.focus();
        }
    };

    // Started, and focused, when it's shown.
    Effect::new(move |_| {
        if active.get() {
            // After the panel has laid out.
            set_timeout(
                move || {
                    start();
                    focus();
                },
                std::time::Duration::from_millis(30),
            );
        }
    });

    // The panel's own buttons (tabs, full height) give the keyboard back.
    Effect::new(move |_| {
        refocus.track();
        if active.get_untracked() {
            set_timeout(focus, std::time::Duration::from_millis(10));
        }
    });

    // The box's size decides the grid; a change resizes the program's screen.
    let observer = StoredValue::new_local(None::<web_sys::ResizeObserver>);
    Effect::new(move |_| {
        let Some(el) = screen.get() else { return };
        if observer.with_value(Option::is_some) {
            return;
        }
        let on_resize = Closure::<dyn Fn()>::new(move || {
            measure_cell();
            let Some((cols, rows)) = fit() else { return };
            if state.size.get_untracked() == (cols, rows) {
                return;
            }
            state.size.set((cols, rows));
            if let Some(id) = state.id.get_untracked() {
                spawn_local(async move {
                    let _ = api::terminal_resize(id, cols, rows).await;
                });
            }
        });
        if let Ok(ro) = web_sys::ResizeObserver::new(on_resize.as_ref().unchecked_ref()) {
            ro.observe(&el);
            observer.set_value(Some(ro));
        }
        on_resize.forget();
    });

    let on_key = move |ev: web_sys::KeyboardEvent| {
        let key = ev.key();
        let ctrl = ev.ctrl_key() || ev.meta_key();
        // Ctrl+` belongs to the panel; Ctrl+V and Ctrl+Shift+V paste.
        if ctrl && ev.code() == "Backquote" {
            return;
        }
        if ctrl && key.eq_ignore_ascii_case("v") {
            return;
        }
        // Ctrl+Shift+Space is the panel's talk key (ticket 187).
        if ctrl && ev.shift_key() && key == " " {
            return;
        }
        if ctrl && ev.shift_key() && key.eq_ignore_ascii_case("c") {
            ev.prevent_default();
            if let Some(doc) = document().dyn_ref::<web_sys::HtmlDocument>() {
                let _ = doc.exec_command("copy");
            }
            return;
        }
        if state.exited.get_untracked() {
            ev.prevent_default();
            if key == "Enter" {
                state.reset();
                start();
            }
            return;
        }
        let Some(id) = state.id.get_untracked() else {
            return;
        };
        let (app_cursor, _, _) = state.modes.get_untracked();
        if let Some(bytes) = key_bytes(&key, ctrl, ev.alt_key(), ev.shift_key(), app_cursor) {
            ev.prevent_default();
            sender.send(id, bytes);
        }
    };
    let on_paste = move |ev: web_sys::ClipboardEvent| {
        ev.prevent_default();
        let Some(id) = state.id.get_untracked() else {
            return;
        };
        let Some(text) = ev
            .clipboard_data()
            .and_then(|d| d.get_data("text/plain").ok())
        else {
            return;
        };
        let (_, bracketed, _) = state.modes.get_untracked();
        sender.send(id, paste_bytes(&text, bracketed));
    };
    let on_wheel = move |ev: web_sys::WheelEvent| {
        let Some(id) = state.id.get_untracked() else {
            return;
        };
        ev.prevent_default();
        let notches = if ev.delta_y() < 0.0 { 3 } else { -3 };
        let (app_cursor, _, alt) = state.modes.get_untracked();
        if alt {
            // A full-screen program scrolls itself: send it arrows.
            let key = if notches > 0 { "ArrowUp" } else { "ArrowDown" };
            if let Some(bytes) = key_bytes(key, false, false, false, app_cursor) {
                sender.send(id, bytes.repeat(3));
            }
        } else {
            spawn_local(async move {
                let _ = api::terminal_scroll(id, notches).await;
            });
        }
    };
    // A click puts the keyboard in the terminal, unless it made a selection.
    let on_mouseup = move |_| {
        let selected = window()
            .get_selection()
            .ok()
            .flatten()
            .is_some_and(|s| !s.is_collapsed());
        if !selected {
            focus();
        }
    };

    let cursor_style = move || {
        let (cw, ch) = cell.get();
        let c = state.cursor.get();
        format!(
            "left:{}px;top:{}px;width:{}px;height:{}px;",
            PAD_X + f64::from(c.x) * cw,
            PAD_Y + f64::from(c.y) * ch,
            cw.max(1.0),
            ch.max(1.0)
        )
    };
    let row_height = move || format!("height:{}px;", cell.get().1.max(1.0));

    view! {
        <div
            node_ref=screen
            class="term relative h-full w-full overflow-hidden px-2 py-1 font-mono text-[13px] leading-[1.25]"
            style="--term-fg:#d4d4d4;--term-bg:#1e1e1e;color:var(--term-fg);background:var(--term-bg);"
            on:wheel=on_wheel
            on:mouseup=on_mouseup
        >
            <span node_ref=measure class="invisible absolute whitespace-pre" aria-hidden="true">
                "WWWWWWWWWW"
            </span>
            <textarea
                node_ref=input
                class="absolute left-0 top-0 h-px w-px resize-none opacity-0"
                aria-label=move || match state.kind {
                    TermKind::Shell => "Terminal input",
                    TermKind::Ai => "AI terminal input",
                }
                autocapitalize="off"
                autocomplete="off"
                spellcheck="false"
                on:keydown=on_key
                on:paste=on_paste
                on:input=move |ev| {
                    if let Some(el) = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlTextAreaElement>().ok()) {
                        el.set_value("");
                    }
                }
                on:focus=move |_| focused.set(true)
                on:blur=move |_| focused.set(false)
            ></textarea>
            {move || {
                state
                    .rows
                    .get()
                    .into_iter()
                    .map(|row| {
                        view! {
                            <div class="whitespace-pre" style=row_height>
                                {move || {
                                    row.get()
                                        .into_iter()
                                        .map(|run| {
                                            let style = run_style(&run);
                                            view! { <span style=style>{run.text}</span> }
                                        })
                                        .collect_view()
                                }}
                            </div>
                        }
                    })
                    .collect_view()
            }}
            <Show when=move || state.cursor.get().visible && state.id.get().is_some()>
                <div
                    class=move || {
                        if focused.get() {
                            "pointer-events-none absolute bg-[#d4d4d4] opacity-60"
                        } else {
                            "pointer-events-none absolute border border-[#d4d4d4] opacity-60"
                        }
                    }
                    style=cursor_style
                ></div>
            </Show>
            <Show when=move || state.exited.get()>
                <div class="absolute bottom-1 left-2 text-[#29b8db]">
                    "[exited — press Enter to start again]"
                </div>
            </Show>
        </div>
    }
}
