//! Markdown, turned into the HTML that goes inside a page.

use std::collections::HashMap;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};

/// A rendered document: the body, and the title to put in the tab.
pub struct Rendered {
  /// The first `<h1>`, or the file name when the document has none.
  pub title: String,
  /// The document body as HTML, without any surrounding page.
  pub body: String,
}

/// GitHub's markdown, as close as a reader expects: a README that renders there
/// should render the same here. Smart punctuation is the one addition of our
/// own — the pages are set as a printed sheet, and straight quotes look wrong.
fn options() -> Options {
  Options::ENABLE_TABLES
    | Options::ENABLE_FOOTNOTES
    | Options::ENABLE_STRIKETHROUGH
    | Options::ENABLE_TASKLISTS
    | Options::ENABLE_SMART_PUNCTUATION
    | Options::ENABLE_HEADING_ATTRIBUTES
    | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
    | Options::ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
}

pub fn render(source: &str, name: &str) -> Rendered {
  let mut events: Vec<Event<'_>> = Parser::new_ext(source, options()).collect();
  drop_metadata(&mut events);
  anchor_headings(&mut events);
  #[cfg(feature = "highlight")]
  highlight_code(&mut events);

  let mut body = String::new();
  html::push_html(&mut body, events.iter().cloned());

  Rendered {
    title: heading(&events).unwrap_or_else(|| name.to_string()),
    body,
  }
}

/// Replace each fenced block whose language syntect recognises with the HTML it
/// produces. A block it does not recognise is left exactly as it was, so the
/// plain rendering stays the fallback rather than an error path.
#[cfg(feature = "highlight")]
fn highlight_code(events: &mut Vec<Event<'_>>) {
  use pulldown_cmark::{CodeBlockKind, CowStr};

  let mut out: Vec<Event<'_>> = Vec::with_capacity(events.len());
  let mut rest = std::mem::take(events).into_iter().peekable();

  while let Some(event) = rest.next() {
    let Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) = &event else {
      out.push(event);
      continue;
    };
    // The language is the first word of the info string: ```rust,ignore.
    let lang = lang
      .split([',', ' '])
      .next()
      .unwrap_or_default()
      .to_string();

    let mut code = String::new();
    let mut block = Vec::new();
    while let Some(inner) = rest.peek() {
      match inner {
        Event::End(TagEnd::CodeBlock) => break,
        Event::Text(text) => code.push_str(text),
        _ => {}
      }
      block.push(rest.next().expect("peeked"));
    }
    let end = rest.next();

    match crate::highlight::code(&code, &lang) {
      Some(highlighted) => {
        out.push(Event::Html(CowStr::from(format!(
          "<pre class=\"hl language-{lang}\"><code>{highlighted}</code></pre>\n"
        ))));
      }
      None => {
        out.push(event);
        out.extend(block);
        out.extend(end);
      }
    }
  }

  *events = out;
}

/// Front matter is configuration for some other tool. It is asked for only so
/// that it can be recognised and thrown away rather than printed as a heading.
fn drop_metadata(events: &mut Vec<Event<'_>>) {
  let mut inside = false;
  events.retain(|event| match event {
    Event::Start(Tag::MetadataBlock(_)) => {
      inside = true;
      false
    }
    Event::End(TagEnd::MetadataBlock(_)) => {
      inside = false;
      false
    }
    _ => !inside,
  });
}

/// Give every heading an `id`, so the `#installation` links a README is full of
/// land somewhere. An id the author wrote by hand is left as it is.
fn anchor_headings(events: &mut [Event<'_>]) {
  let mut seen: HashMap<String, usize> = HashMap::new();
  let mut anchors: Vec<(usize, String)> = Vec::new();

  for (index, event) in events.iter().enumerate() {
    let Event::Start(Tag::Heading { id: None, .. }) = event else {
      continue;
    };
    let text: String = events[index..]
      .iter()
      .take_while(|event| !matches!(event, Event::End(TagEnd::Heading(_))))
      .filter_map(|event| match event {
        Event::Text(text) | Event::Code(text) => Some(text.as_ref()),
        _ => None,
      })
      .collect();

    anchors.push((index, unique(slug(&text), &mut seen)));
  }

  for (index, anchor) in anchors {
    if let Event::Start(Tag::Heading { id, .. }) = &mut events[index] {
      *id = Some(anchor.into());
    }
  }
}

/// `Getting Started?` becomes `getting-started`. Letters and digits survive in
/// any alphabet; everything else becomes a separator.
fn slug(text: &str) -> String {
  let mut out = String::new();
  for ch in text.chars() {
    if ch.is_alphanumeric() {
      out.extend(ch.to_lowercase());
    } else if !out.ends_with('-') {
      out.push('-');
    }
  }
  let trimmed = out.trim_matches('-');
  if trimmed.is_empty() {
    "section".to_string()
  } else {
    trimmed.to_string()
  }
}

/// Two headings with the same words would otherwise share one anchor, and the
/// second would be unreachable.
fn unique(anchor: String, seen: &mut HashMap<String, usize>) -> String {
  let count = seen.entry(anchor.clone()).or_insert(0);
  *count += 1;
  if *count == 1 {
    anchor
  } else {
    format!("{anchor}-{}", *count - 1)
  }
}

/// The text of the first top-level heading, which is what a browser tab wants.
fn heading(events: &[Event<'_>]) -> Option<String> {
  let start = events.iter().position(|event| {
    matches!(
      event,
      Event::Start(Tag::Heading {
        level: HeadingLevel::H1,
        ..
      })
    )
  })?;

  let text: String = events[start..]
    .iter()
    .take_while(|event| !matches!(event, Event::End(TagEnd::Heading(_))))
    .filter_map(|event| match event {
      Event::Text(text) | Event::Code(text) => Some(text.as_ref()),
      _ => None,
    })
    .collect();

  (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn body(source: &str) -> String {
    render(source, "doc.md").body
  }

  /// The words on the page, with the markup taken off — highlighting splits a
  /// line into spans, so the text has to be compared without them.
  fn words(html: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for ch in html.chars() {
      match ch {
        '<' => inside = true,
        '>' => inside = false,
        _ if !inside => out.push(ch),
        _ => {}
      }
    }
    out
      .replace("&lt;", "<")
      .replace("&gt;", ">")
      .replace("&amp;", "&")
  }

  #[test]
  fn renders_a_paragraph() {
    assert_eq!(body("hello"), "<p>hello</p>\n");
  }

  #[test]
  fn the_title_is_the_first_heading() {
    assert_eq!(render("# Reading\n\ntext", "doc.md").title, "Reading");
  }

  #[test]
  fn a_document_without_a_heading_is_titled_by_its_file_name() {
    assert_eq!(render("just text", "notes.md").title, "notes.md");
  }

  #[test]
  fn renders_a_table() {
    let html = body("| a | b |\n| - | - |\n| 1 | 2 |");
    assert!(html.contains("<table>"), "{html}");
    assert!(html.contains("<th>a</th>"), "{html}");
    assert!(html.contains("<td>2</td>"), "{html}");
  }

  #[test]
  fn renders_strikethrough() {
    assert!(body("~~gone~~").contains("<del>gone</del>"));
  }

  #[test]
  fn renders_a_task_list() {
    let html = body("- [x] done\n- [ ] todo");
    assert!(
      html.contains(r#"<input disabled="" type="checkbox" checked=""#),
      "{html}"
    );
    assert_eq!(html.matches("type=\"checkbox\"").count(), 2, "{html}");
  }

  #[test]
  fn renders_a_footnote() {
    let html = body("text[^a]\n\n[^a]: the note");
    assert!(html.contains("footnote-reference"), "{html}");
    assert!(html.contains("the note"), "{html}");
  }

  #[test]
  fn straightens_quotes_into_typography() {
    // The whole project is set as a printed sheet; "quotes" should look it.
    let html = body(r#""quoted" and -- dashed"#);
    assert!(
      html.contains('\u{201c}') && html.contains('\u{201d}'),
      "{html}"
    );
    assert!(html.contains('\u{2013}'), "{html}");
  }

  #[test]
  fn front_matter_is_swallowed_rather_than_printed() {
    let html = body("---\ntitle: Notes\ndraft: true\n---\n\nthe body");
    assert!(!html.contains("draft"), "{html}");
    assert!(!html.contains("<hr"), "{html}");
    assert_eq!(html.trim(), "<p>the body</p>");
  }

  #[test]
  fn plus_delimited_front_matter_is_swallowed_too() {
    let html = body("+++\ntitle = \"Notes\"\n+++\n\nthe body");
    assert!(!html.contains("title"), "{html}");
    assert_eq!(html.trim(), "<p>the body</p>");
  }

  #[test]
  fn front_matter_does_not_become_the_title() {
    // The metadata is skipped whole, so the first real heading still wins.
    let doc = render("---\ntitle: Meta\n---\n\n# Real\n", "doc.md");
    assert_eq!(doc.title, "Real");
  }

  #[test]
  fn headings_get_anchors_so_fragment_links_work() {
    let html = body("## Getting Started\n");
    assert!(html.contains(r#"<h2 id="getting-started">"#), "{html}");
  }

  #[test]
  fn an_explicit_heading_id_is_left_alone() {
    let html = body("## Getting Started {#start}\n");
    assert!(html.contains(r#"<h2 id="start">"#), "{html}");
  }

  #[test]
  fn anchors_are_unique_when_headings_repeat() {
    let html = body("## Notes\n\n## Notes\n");
    assert!(html.contains(r#"id="notes""#), "{html}");
    assert!(html.contains(r#"id="notes-1""#), "{html}");
  }

  #[test]
  fn anchors_drop_punctuation_and_keep_unicode() {
    let html = body("## Что нового?\n");
    assert!(html.contains(r#"id="что-нового""#), "{html}");
  }

  #[test]
  fn a_heading_of_only_punctuation_still_gets_some_anchor() {
    let html = body("## ???\n");
    assert!(html.contains("id=\""), "{html}");
  }

  #[test]
  fn a_fenced_block_keeps_its_code_whatever_is_compiled_in() {
    let html = body("```rust\nlet x = 1;\n```\n");
    assert!(html.contains("<pre"), "{html}");
    assert!(words(&html).contains("let x = 1;"), "{html}");
  }

  #[test]
  fn a_fence_in_an_unknown_language_is_left_plain() {
    let html = body("```nosuchlanguage\nwhatever\n```\n");
    assert!(words(&html).contains("whatever"), "{html}");
    assert!(!html.contains("<span class=\"hl-"), "{html}");
  }

  #[test]
  fn an_unfenced_block_is_left_plain() {
    let html = body("    indented code\n");
    assert!(words(&html).contains("indented code"), "{html}");
    assert!(!html.contains("<span class=\"hl-"), "{html}");
  }

  #[test]
  fn code_is_escaped_rather_than_interpreted() {
    let html = body("```rust\nlet s = \"<b>\";\n```\n");
    assert!(!html.contains("<b>"), "{html}");
    assert!(html.contains("&lt;b&gt;"), "{html}");
    assert!(words(&html).contains("<b>"), "{html}");
  }

  #[cfg(feature = "highlight")]
  #[test]
  fn a_known_language_is_highlighted_into_spans() {
    let html = body("```rust\nlet x = 1;\n```\n");
    assert!(html.contains("<span class=\"hl-"), "{html}");
  }

  #[cfg(not(feature = "highlight"))]
  #[test]
  fn without_the_feature_a_fence_keeps_its_language_class() {
    let html = body("```rust\nlet x = 1;\n```\n");
    assert!(html.contains(r#"<code class="language-rust">"#), "{html}");
    assert!(!html.contains("<span class=\"hl-"), "{html}");
  }

  #[test]
  fn raw_html_passes_through() {
    // Your own README on your own machine; stripping what you wrote would be
    // the wrong kind of safe.
    assert!(body("<kbd>ctrl</kbd>").contains("<kbd>ctrl</kbd>"));
  }
}
