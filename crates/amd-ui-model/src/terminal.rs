//! The bottom panel's terminal (ticket 185): what the backend sends the page
//! to draw, and how the page turns keys into the bytes a terminal program
//! expects. The screen itself is `alacritty_terminal`'s, in `amd-ui`; this
//! side has no terminal state of its own.

use serde::{Deserialize, Serialize};

/// Which program a panel tab runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TermKind {
    /// The user's shell.
    Shell,
    /// The AI command from Preferences (`claude` by default).
    Ai,
}

/// A stretch of a row drawn the same way.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermRun {
    pub text: String,
    /// CSS colour, or empty for the theme's own foreground.
    #[serde(default)]
    pub fg: String,
    /// CSS colour, or empty for none.
    #[serde(default)]
    pub bg: String,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub underline: bool,
    #[serde(default)]
    pub dim: bool,
    #[serde(default)]
    pub strike: bool,
}

/// One screen row that changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermRow {
    /// 0 is the top of the screen.
    pub y: u16,
    pub runs: Vec<TermRun>,
}

/// Where the cursor is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermCursor {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
}

/// What changed on a terminal's screen since the last frame.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermFrame {
    pub id: u32,
    pub cols: u16,
    pub lines: u16,
    /// Only the rows that changed; every row when `full`.
    pub rows: Vec<TermRow>,
    pub full: bool,
    pub cursor: TermCursor,
    /// Arrow keys send `ESC O A…` rather than `ESC [ A…`.
    pub app_cursor: bool,
    /// Pasted text is wrapped in `ESC [200~ … ESC [201~`.
    pub bracketed_paste: bool,
    /// A full-screen program (an editor, `top`, Claude Code) has the screen;
    /// the mouse wheel sends arrow keys instead of scrolling back.
    pub alt_screen: bool,
}

/// A speech model, as Preferences lists it (ticket 187).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechModel {
    /// `tiny.en` or `base.en`.
    pub id: String,
    pub size_mb: u32,
    pub installed: bool,
}

/// Where speech to text stands on this machine.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechStatus {
    /// The "Use speech to text" switch.
    pub enabled: bool,
    /// The model dictation uses, when one is installed.
    pub model: Option<String>,
    pub models: Vec<SpeechModel>,
    /// The default microphone's name, if there is one.
    pub microphone: Option<String>,
}

impl SpeechStatus {
    /// Dictation can be offered: switched on, with a model installed.
    pub fn ready(&self) -> bool {
        self.enabled && self.model.is_some()
    }
}

/// Where read aloud stands (ticket 286).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadAloudStatus {
    /// The "Read aloud" switch.
    pub enabled: bool,
    /// The system speech found (`speech-dispatcher`, `macOS speech`), if any.
    pub engine: Option<String>,
    /// Something is being read now.
    pub speaking: bool,
}

impl ReadAloudStatus {
    /// The 🔊 button can be offered.
    pub fn ready(&self) -> bool {
        self.enabled && self.engine.is_some()
    }
}

/// How a model's download is going.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeechProgress {
    pub model: String,
    pub done: u64,
    pub total: u64,
    pub finished: bool,
    pub error: Option<String>,
}

/// The bytes a key sends, or `None` for a key the terminal doesn't take
/// (so the page leaves it alone). `key` is the DOM `KeyboardEvent.key`.
pub fn key_bytes(
    key: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    app_cursor: bool,
) -> Option<String> {
    let csi_or_ss3 = |c: char| {
        if app_cursor {
            format!("\x1bO{c}")
        } else {
            format!("\x1b[{c}")
        }
    };
    let base = match key {
        "Enter" => "\r".to_string(),
        "Backspace" => if ctrl { "\x08" } else { "\x7f" }.to_string(),
        "Tab" => if shift { "\x1b[Z" } else { "\t" }.to_string(),
        "Escape" => "\x1b".to_string(),
        "ArrowUp" => csi_or_ss3('A'),
        "ArrowDown" => csi_or_ss3('B'),
        "ArrowRight" => csi_or_ss3('C'),
        "ArrowLeft" => csi_or_ss3('D'),
        "Home" => csi_or_ss3('H'),
        "End" => csi_or_ss3('F'),
        "Insert" => "\x1b[2~".to_string(),
        "Delete" => "\x1b[3~".to_string(),
        "PageUp" => "\x1b[5~".to_string(),
        "PageDown" => "\x1b[6~".to_string(),
        "F1" => "\x1bOP".to_string(),
        "F2" => "\x1bOQ".to_string(),
        "F3" => "\x1bOR".to_string(),
        "F4" => "\x1bOS".to_string(),
        "F5" => "\x1b[15~".to_string(),
        "F6" => "\x1b[17~".to_string(),
        "F7" => "\x1b[18~".to_string(),
        "F8" => "\x1b[19~".to_string(),
        "F9" => "\x1b[20~".to_string(),
        "F10" => "\x1b[21~".to_string(),
        "F11" => "\x1b[23~".to_string(),
        "F12" => "\x1b[24~".to_string(),
        _ => {
            let mut chars = key.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                // A named key we don't send (Shift, CapsLock, Meta…).
                return None;
            }
            if ctrl {
                control(c)?.to_string()
            } else {
                c.to_string()
            }
        }
    };
    Some(if alt { format!("\x1b{base}") } else { base })
}

/// Ctrl with a key: the control character it stands for.
fn control(c: char) -> Option<char> {
    let byte = match c.to_ascii_lowercase() {
        c @ 'a'..='z' => c as u8 - b'a' + 1,
        '@' | ' ' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    };
    Some(byte as char)
}

/// Pasted text as the terminal should get it: line endings as Enter, and
/// wrapped when the program asked for bracketed paste.
pub fn paste_bytes(text: &str, bracketed: bool) -> String {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        // A pasted end marker would end the paste early.
        format!("\x1b[200~{}\x1b[201~", text.replace("\x1b[201~", ""))
    } else {
        text
    }
}

/// How many columns and rows fit a panel of `width` × `height` pixels with
/// cells of `cell_w` × `cell_h`; never less than 2 × 1.
pub fn grid_size(width: f64, height: f64, cell_w: f64, cell_h: f64) -> (u16, u16) {
    let fit = |space: f64, cell: f64, least: u16| {
        if cell <= 0.0 {
            return least;
        }
        ((space / cell).floor() as i64).clamp(least as i64, u16::MAX as i64) as u16
    };
    (fit(width, cell_w, 2), fit(height, cell_h, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: &str) -> Option<String> {
        key_bytes(k, false, false, false, false)
    }

    #[test]
    fn printable_keys_are_themselves() {
        assert_eq!(key("a").as_deref(), Some("a"));
        assert_eq!(key("A").as_deref(), Some("A"));
        assert_eq!(key("é").as_deref(), Some("é"));
        assert_eq!(key(" ").as_deref(), Some(" "));
        assert_eq!(key("Shift"), None);
        assert_eq!(key("CapsLock"), None);
    }

    #[test]
    fn editing_keys() {
        assert_eq!(key("Enter").as_deref(), Some("\r"));
        assert_eq!(key("Backspace").as_deref(), Some("\x7f"));
        assert_eq!(key("Tab").as_deref(), Some("\t"));
        assert_eq!(
            key_bytes("Tab", false, false, true, false).as_deref(),
            Some("\x1b[Z")
        );
        assert_eq!(key("Escape").as_deref(), Some("\x1b"));
        assert_eq!(key("Delete").as_deref(), Some("\x1b[3~"));
        assert_eq!(key("PageDown").as_deref(), Some("\x1b[6~"));
        assert_eq!(key("F1").as_deref(), Some("\x1bOP"));
        assert_eq!(key("F12").as_deref(), Some("\x1b[24~"));
    }

    #[test]
    fn arrows_follow_the_cursor_mode() {
        assert_eq!(key("ArrowUp").as_deref(), Some("\x1b[A"));
        assert_eq!(key("ArrowLeft").as_deref(), Some("\x1b[D"));
        assert_eq!(
            key_bytes("ArrowUp", false, false, false, true).as_deref(),
            Some("\x1bOA")
        );
        assert_eq!(
            key_bytes("Home", false, false, false, true).as_deref(),
            Some("\x1bOH")
        );
    }

    #[test]
    fn ctrl_and_alt() {
        assert_eq!(
            key_bytes("c", true, false, false, false).as_deref(),
            Some("\x03")
        );
        assert_eq!(
            key_bytes("C", true, false, true, false).as_deref(),
            Some("\x03")
        );
        assert_eq!(
            key_bytes("r", true, false, false, false).as_deref(),
            Some("\x12")
        );
        assert_eq!(
            key_bytes("[", true, false, false, false).as_deref(),
            Some("\x1b")
        );
        assert_eq!(
            key_bytes(" ", true, false, false, false).as_deref(),
            Some("\0")
        );
        assert_eq!(
            key_bytes("b", false, true, false, false).as_deref(),
            Some("\x1bb")
        );
        assert_eq!(
            key_bytes("ArrowUp", false, true, false, false).as_deref(),
            Some("\x1b\x1b[A")
        );
        assert_eq!(key_bytes("é", true, false, false, false), None);
    }

    #[test]
    fn pastes() {
        assert_eq!(paste_bytes("a\nb\r\nc", false), "a\rb\rc");
        assert_eq!(paste_bytes("hi", true), "\x1b[200~hi\x1b[201~");
        assert_eq!(paste_bytes("x\x1b[201~y", true), "\x1b[200~xy\x1b[201~");
    }

    #[test]
    fn the_grid_fits_the_panel() {
        assert_eq!(grid_size(800.0, 200.0, 8.0, 20.0), (100, 10));
        assert_eq!(grid_size(805.0, 219.0, 8.0, 20.0), (100, 10));
        assert_eq!(grid_size(0.0, 0.0, 8.0, 20.0), (2, 1));
        assert_eq!(grid_size(100.0, 100.0, 0.0, 0.0), (2, 1));
    }
}
