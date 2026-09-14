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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn wraps_text_in_an_sgr_sequence_when_colour_is_on() {
    assert_eq!(paint(true, "32", "200"), "\x1b[32m200\x1b[0m");
    assert_eq!(
      paint(true, "38;2;192;74;47", "▌"),
      "\x1b[38;2;192;74;47m▌\x1b[0m"
    );
  }

  #[test]
  fn hands_the_text_back_untouched_when_colour_is_off() {
    assert_eq!(paint(false, "32", "200"), "200");
    assert_eq!(paint(false, "1", ""), "");
  }

  #[test]
  fn painting_never_changes_what_the_text_says() {
    // The escapes bracket the text; whatever a pipe strips, the words survive.
    for on in [true, false] {
      assert!(paint(on, "2", "root").contains("root"));
    }
  }

  #[test]
  fn a_pipe_is_never_painted() {
    // `enabled` is the whole decision: not a terminal means no escapes, and a
    // terminal still defers to NO_COLOR. Reading the environment is left to the
    // caller so this stays a pure function.
    assert!(!enabled(false));
  }

  #[test]
  fn a_stream_is_asked_once_and_keeps_its_answer() {
    assert_eq!(stdout(), stdout());
    assert_eq!(stderr(), stderr());
  }
}
