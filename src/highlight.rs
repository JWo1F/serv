//! Server-side syntax highlighting, compiled in only with `--features highlight`.
//!
//! syntect carries a dump of TextMate grammars and a regex engine to run them —
//! several times the size of serv itself, which is why it is not the default.
//! The output is class-based rather than syntect's inline theme colours, so the
//! palette stays in `style.css` and the code follows the page into dark mode.

use std::sync::OnceLock;

use syntect::html::{ClassStyle, ClassedHTMLGenerator};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

/// The prefix every generated class carries, so highlighting can never collide
/// with a class name in the document's own raw HTML.
const PREFIX: &str = "hl-";

const STYLE: ClassStyle = ClassStyle::SpacedPrefixed { prefix: PREFIX };

/// Loading the grammar dump costs a few milliseconds; it is done once.
fn syntaxes() -> &'static SyntaxSet {
  static SET: OnceLock<SyntaxSet> = OnceLock::new();
  SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// Highlight `code` as `lang`, or hand back `None` when that language is not one
/// syntect knows — the caller then renders the block plain.
pub fn code(code: &str, lang: &str) -> Option<String> {
  let syntaxes = syntaxes();
  let syntax = syntaxes
    .find_syntax_by_token(lang)
    .or_else(|| syntaxes.find_syntax_by_extension(lang))?;

  let mut generator = ClassedHTMLGenerator::new_with_class_style(syntax, syntaxes, STYLE);
  for line in LinesWithEndings::from(code) {
    // A malformed line is not worth losing the whole block over.
    if generator
      .parse_html_for_line_which_includes_newline(line)
      .is_err()
    {
      return None;
    }
  }
  Some(generator.finalize())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn highlights_a_language_it_knows() {
    let html = code("let x = 1;\n", "rust").expect("rust is a known language");
    assert!(html.contains("<span class=\"hl-"), "{html}");
    assert!(html.contains("let"), "{html}");
  }

  #[test]
  fn a_language_it_does_not_know_is_not_highlighted() {
    assert!(code("whatever\n", "nosuchlanguage").is_none());
  }

  #[test]
  fn an_extension_names_a_language_too() {
    // A fence is as likely to say ```rs as ```rust.
    assert!(code("let x = 1;\n", "rs").is_some());
  }

  #[test]
  fn markup_in_the_code_is_escaped() {
    let html = code("let s = \"<b>\";\n", "rust").expect("rust");
    assert!(!html.contains("<b>"), "{html}");
    assert!(html.contains("&lt;b&gt;"), "{html}");
  }

  #[test]
  fn an_empty_block_is_still_valid_output() {
    assert_eq!(code("", "rust"), Some(String::new()));
  }
}
