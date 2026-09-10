//! Whether to write ANSI escapes, decided once per stream.
//!
//! env_logger used to strip escapes on our behalf when the output was not a
//! terminal, which meant carrying `anstream` and, through the same default
//! feature set, a regex engine and a datetime library serv never called. The
//! decision is cheap to make here, and each stream gets its own answer: the
//! banner goes to stdout, request logs to stderr, and one can be a pipe while
//! the other is a terminal.

use std::io::IsTerminal;
use std::sync::OnceLock;

pub fn stdout() -> bool {
  static ON: OnceLock<bool> = OnceLock::new();
  *ON.get_or_init(|| enabled(std::io::stdout().is_terminal()))
}

pub fn stderr() -> bool {
  static ON: OnceLock<bool> = OnceLock::new();
  *ON.get_or_init(|| enabled(std::io::stderr().is_terminal()))
}

fn enabled(is_terminal: bool) -> bool {
  is_terminal && std::env::var_os("NO_COLOR").is_none()
}

/// Wrap `text` in an SGR sequence, or hand it back untouched.
pub fn paint(on: bool, code: &str, text: &str) -> String {
  if on {
    format!("\x1b[{code}m{text}\x1b[0m")
  } else {
    text.to_string()
  }
}
