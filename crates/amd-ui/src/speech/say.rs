//! Reading text aloud (ticket 286) with the system's own speech:
//! speech-dispatcher's `spd-say` on Linux, `say` on macOS. Nothing is
//! downloaded and nothing leaves the machine; the voice is whatever the
//! system's speech is set up with.

use std::io;
use std::path::Path;
use std::process::{Child, Command, Stdio};

/// The most read in one go, so a long output isn't read for minutes.
pub const MAX_CHARS: usize = 4000;

/// The system speech amdui found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    /// speech-dispatcher, through `spd-say`.
    SpdSay,
    /// macOS's `say`.
    Say,
}

impl Engine {
    /// What this machine has, speech-dispatcher first.
    pub fn find() -> Option<Engine> {
        if on_path("spd-say") {
            Some(Engine::SpdSay)
        } else if cfg!(target_os = "macos") && on_path("say") {
            Some(Engine::Say)
        } else {
            None
        }
    }

    /// The name Preferences shows.
    pub fn name(self) -> &'static str {
        match self {
            Engine::SpdSay => "speech-dispatcher",
            Engine::Say => "macOS speech",
        }
    }

    /// The command that says `text` and returns once it's said (or stopped).
    fn command(self, text: &str) -> Command {
        let mut command = match self {
            // One message, so `-S` stops all of it; `--` so text starting
            // with a dash isn't an option.
            Engine::SpdSay => {
                let mut c = Command::new("spd-say");
                c.args(["--wait", "--"]);
                c
            }
            Engine::Say => {
                let mut c = Command::new("say");
                c.arg("--");
                c
            }
        };
        command.arg(text);
        command
    }

    /// Starts saying `text`.
    pub fn speak(self, text: &str) -> io::Result<Child> {
        self.command(text)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }

    /// Stops `child` saying anything more. Killing `spd-say` only ends the
    /// client: the speech server would carry on, so it's told to stop the
    /// message being spoken (`-S`, not `-C`, which cancels everyone's, a
    /// screen reader's included).
    pub fn stop(self, child: &mut Child) {
        if self == Engine::SpdSay {
            let _ = Command::new("spd-say")
                .arg("-S")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

pub(crate) fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| is_executable(&dir.join(program)))
    })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Terminal text as something worth hearing: drawing (box lines,
/// spinners), list and quote marks and markdown's `**` and backticks go;
/// so do an AI tool's tool calls (`● Bash(…)`) and their output (`⎿ …` and
/// what's indented under it). Capped at [`MAX_CHARS`], at a sentence's end
/// where there is one.
pub fn speakable(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut in_tool = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            in_tool = false;
            if lines.last().is_some_and(|l| !l.is_empty()) {
                lines.push(String::new());
            }
            continue;
        }
        if line.starts_with('⎿') || is_tool_call(line) {
            in_tool = true;
            continue;
        }
        if in_tool && raw.starts_with(char::is_whitespace) {
            continue;
        }
        in_tool = false;
        if !line.chars().any(char::is_alphanumeric) {
            continue;
        }
        let clean = clean_line(line);
        if !clean.is_empty() {
            lines.push(clean);
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    cap(&lines.join("\n"))
}

/// `● Bash(ls)`, `● Read(src/main.rs)`: a marker, then a word and `(`.
fn is_tool_call(line: &str) -> bool {
    let Some(rest) = line.strip_prefix('●') else {
        return false;
    };
    let rest = rest.trim_start();
    let name: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
    !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_uppercase())
        && rest[name.len()..].starts_with('(')
}

fn clean_line(line: &str) -> String {
    let mut s = line;
    // Leading marks, possibly several (`> - item`).
    loop {
        let before = s;
        for mark in ["●", "•", "◦", "▪", "✻", "✶", "✳", "*", "-", ">", "#"] {
            if let Some(rest) = s.strip_prefix(mark)
                && (rest.is_empty() || rest.starts_with(char::is_whitespace) || mark == "#")
            {
                s = rest.trim_start();
            }
        }
        if s == before {
            break;
        }
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '`' | '*' => {}
            // Box drawing and blocks read as nothing.
            '\u{2500}'..='\u{259F}' => out.push(' '),
            _ => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cap(text: &str) -> String {
    if text.chars().count() <= MAX_CHARS {
        return text.to_string();
    }
    let cut: String = text.chars().take(MAX_CHARS).collect();
    match cut.rfind(['.', '!', '?']) {
        Some(end) if end > MAX_CHARS / 2 => cut[..=end].to_string(),
        _ => cut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_and_marks_go() {
        let text =
            "╭──────╮\n│ hi │\n╰──────╯\n● The **answer** is `42`.\n- one\n> quoted\n## Heading";
        assert_eq!(
            speakable(text),
            "hi\nThe answer is 42.\none\nquoted\nHeading"
        );
    }

    #[test]
    fn tool_calls_and_their_output_are_left_out() {
        let text = "● I'll look.\n\n● Bash(ls -la)\n  ⎿  total 8\n     drwx file\n     more\n\n● Read(src/main.rs)\n  ⎿  Read 20 lines\n\n● Done: it's fine.";
        assert_eq!(speakable(text), "I'll look.\n\nDone: it's fine.");
    }

    #[test]
    fn a_word_in_brackets_is_not_a_tool_call() {
        assert!(!is_tool_call("● the value (in bytes) is 4"));
        assert!(is_tool_call("● Bash(cargo test)"));
        assert!(is_tool_call("● Update(src/lib.rs)"));
    }

    #[test]
    fn plain_shell_output_is_kept() {
        assert_eq!(speakable("hello\nworld"), "hello\nworld");
        assert_eq!(
            speakable("-rw-r--r-- 1 me me 12 file"),
            "-rw-r--r-- 1 me me 12 file"
        );
    }

    #[test]
    fn spinners_and_rules_only_say_nothing() {
        assert_eq!(speakable("────────\n⠋\n···"), "");
    }

    #[test]
    fn long_text_is_cut_at_a_sentence() {
        let sentence = "This is a sentence. ";
        let text = sentence.repeat(MAX_CHARS / sentence.len() + 10);
        let said = speakable(&text);
        assert!(said.chars().count() <= MAX_CHARS);
        assert!(said.ends_with('.'));
    }

    #[test]
    fn spd_say_gets_one_message_after_a_double_dash() {
        let command = Engine::SpdSay.command("-x hello");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(command.get_program(), "spd-say");
        assert_eq!(args, ["--wait", "--", "-x hello"]);
    }
}
