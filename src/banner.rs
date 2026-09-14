//! The block of text serv prints on startup.
//!
//! It answers the questions you would otherwise have to guess at: which folder,
//! which address, which page a miss lands on, and whether `/about` will find
//! `about.html`.

use std::fmt::Write as _;
use std::net::SocketAddr;
use std::path::Path;

use crate::config::Config;
use crate::pages::human_size;
use crate::style;

/// serv's madder, warm enough to read on a light terminal and a dark one.
const MADDER: &str = "38;2;192;74;47";
const DIM: &str = "2";
const BOLD: &str = "1";
const GREEN: &str = "32";
const YELLOW: &str = "33";

const LABEL: usize = 11;

pub fn print(config: &Config, addr: SocketAddr, quiet: bool) {
  let paint = Paint::detect();
  let url = format!("http://{addr}/");
  let mut out = String::new();

  // serv▌ — the wordmark, with its cursor still blinking.
  let _ = writeln!(
    out,
    "\n  {}{} {}",
    paint.on(BOLD, "serv"),
    paint.on(MADDER, "▌"),
    paint.on(DIM, env!("CARGO_PKG_VERSION")),
  );

  frame(&mut out, &paint, &url);

  let (folders, files, bytes) = count(&config.root);
  row(&mut out, &paint, "root", &tilde(&config.root));
  row(
    &mut out,
    &paint,
    "contents",
    &format!(
      "{folders} {} {} {files} {} {} {}",
      plural(folders, "folder"),
      paint.on(DIM, "·"),
      plural(files, "file"),
      paint.on(DIM, "·"),
      human_size(bytes),
    ),
  );

  match &config.spa {
    Some(spa) => row(&mut out, &paint, "spa", &present(&paint, spa, "missing")),
    None => {
      let index = config.root.join("index.html");
      let value = if index.is_file() {
        paint.on(GREEN, "index.html")
      } else {
        format!(
          "no index.html {}",
          paint.on(DIM, "· folders get a generated listing")
        )
      };
      row(&mut out, &paint, "index", &value);
    }
  }

  let not_found = match &config.not_found {
    Some(page) => present(&paint, page, "missing, falling back to the built-in page"),
    None => paint.on(DIM, "built-in page"),
  };
  row(&mut out, &paint, "not found", &not_found);

  let urls = if config.ext {
    format!("{} {}", "literal", paint.on(DIM, "· /about.html only"))
  } else {
    format!(
      "{} {}",
      "clean",
      paint.on(DIM, "· /about serves about.html")
    )
  };
  row(&mut out, &paint, "urls", &urls);

  row(
    &mut out,
    &paint,
    "encoding",
    &format!("gzip {}", paint.on(DIM, "· text between 1 KiB and 8 MiB")),
  );
  let logs = if quiet {
    paint.on(DIM, "off (--quiet)")
  } else {
    "on".to_string()
  };
  row(&mut out, &paint, "logs", &logs);

  let _ = writeln!(out, "\n  {}", paint.on(DIM, "ctrl-c to stop"));
  print!("{out}");
}

/// The address, boxed — it is the one line anyone actually needs.
fn frame(out: &mut String, paint: &Paint, url: &str) {
  let inner = url.chars().count() + 4;
  let bar = "─".repeat(inner);
  let edge = |s: &str| paint.on(MADDER, s);

  let _ = writeln!(out, "\n  {}{}{}", edge("╭"), edge(&bar), edge("╮"));
  let _ = writeln!(
    out,
    "  {}  {}  {}",
    edge("│"),
    paint.on(BOLD, url),
    edge("│")
  );
  let _ = writeln!(out, "  {}{}{}\n", edge("╰"), edge(&bar), edge("╯"));
}

fn row(out: &mut String, paint: &Paint, label: &str, value: &str) {
  let _ = writeln!(
    out,
    "  {}  {value}",
    paint.on(DIM, &format!("{label:<LABEL$}"))
  );
}

fn present(paint: &Paint, path: &Path, missing: &str) -> String {
  let name = path
    .file_name()
    .map(|n| n.to_string_lossy().into_owned())
    .unwrap_or_else(|| path.display().to_string());

  if path.is_file() {
    paint.on(GREEN, &name)
  } else {
    format!("{} {}", name, paint.on(YELLOW, &format!("· {missing}")))
  }
}

/// `/Users/you/site` is easier to read as `~/site`.
fn tilde(path: &Path) -> String {
  let shown = path.display().to_string();
  match std::env::var_os("HOME") {
    Some(home) if !home.is_empty() => {
      let home = home.to_string_lossy().into_owned();
      match shown.strip_prefix(&home) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => shown,
      }
    }
    _ => shown,
  }
}

fn plural(n: usize, word: &str) -> String {
  if n == 1 {
    word.to_string()
  } else {
    format!("{word}s")
  }
}

/// One shallow read of the served folder — enough to say what is in there.
fn count(root: &Path) -> (usize, usize, u64) {
  let Ok(entries) = std::fs::read_dir(root) else {
    return (0, 0, 0);
  };

  let mut folders = 0;
  let mut files = 0;
  let mut bytes = 0;

  for entry in entries.flatten() {
    match entry.metadata() {
      Ok(meta) if meta.is_dir() => folders += 1,
      Ok(meta) => {
        files += 1;
        bytes += meta.len();
      }
      Err(_) => {}
    }
  }
  (folders, files, bytes)
}

/// Colour, unless the output is redirected or `NO_COLOR` says otherwise.
struct Paint {
  on: bool,
}

impl Paint {
  fn detect() -> Self {
    Self {
      on: style::stdout(),
    }
  }

  fn on(&self, code: &str, text: &str) -> String {
    style::paint(self.on, code, text)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::fs;
  use std::path::PathBuf;

  fn plain() -> Paint {
    Paint { on: false }
  }

  #[test]
  fn one_of_a_thing_is_singular() {
    assert_eq!(plural(1, "folder"), "folder");
    assert_eq!(plural(0, "folder"), "folders");
    assert_eq!(plural(2, "file"), "files");
  }

  #[test]
  fn counts_the_top_level_of_the_served_folder() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("assets")).unwrap();
    fs::create_dir(dir.path().join("docs")).unwrap();
    fs::write(dir.path().join("index.html"), vec![b'x'; 100]).unwrap();
    fs::write(dir.path().join("app.css"), vec![b'y'; 24]).unwrap();
    // One shallow read, so a file nested inside a folder is not counted.
    fs::write(dir.path().join("docs").join("deep.txt"), vec![b'z'; 999]).unwrap();

    assert_eq!(count(dir.path()), (2, 2, 124));
  }

  #[test]
  fn an_empty_folder_counts_to_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(count(dir.path()), (0, 0, 0));
  }

  #[test]
  fn a_folder_that_cannot_be_read_counts_to_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(count(&dir.path().join("missing")), (0, 0, 0));
  }

  #[test]
  fn a_file_that_exists_is_named_in_green() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("404.html");
    fs::write(&page, "x").unwrap();

    assert_eq!(present(&plain(), &page, "missing"), "404.html");
  }

  #[test]
  fn a_file_that_does_not_exist_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let page = dir.path().join("404.html");

    assert_eq!(present(&plain(), &page, "missing"), "404.html · missing");
  }

  #[test]
  fn shortens_a_path_under_home() {
    // `tilde` reads $HOME, so the expectation is built from it rather than
    // assumed — and the no-HOME case is left to the machine that has none.
    let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) else {
      return;
    };
    let home = home.to_string_lossy().into_owned();

    assert_eq!(tilde(Path::new(&home)), "~");
    assert_eq!(tilde(&PathBuf::from(&home).join("site")), "~/site");
    assert_eq!(
      tilde(&PathBuf::from(&home).join("work/serv")),
      "~/work/serv"
    );
  }

  #[test]
  fn leaves_a_path_outside_home_alone() {
    assert_eq!(tilde(Path::new("/etc")), "/etc");
    assert_eq!(tilde(Path::new("/")), "/");
  }

  #[test]
  fn does_not_shorten_a_sibling_of_home() {
    // `/home/alexandra` starts with `/home/alex` as a string but is a different
    // directory, so only a whole path component may be replaced.
    let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) else {
      return;
    };
    let sibling = format!("{}-backup", home.to_string_lossy());

    assert_eq!(tilde(Path::new(&sibling)), sibling);
  }

  #[test]
  fn the_address_is_boxed_to_the_width_of_the_url() {
    let mut out = String::new();
    frame(&mut out, &plain(), "http://127.0.0.1:8010/");

    let lines: Vec<&str> = out.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 3);
    assert!(lines[1].contains("http://127.0.0.1:8010/"));
    // Every line of the box is the same width, which is what makes it a box.
    let widths: Vec<usize> = lines.iter().map(|l| l.chars().count()).collect();
    assert_eq!(widths[0], widths[1]);
    assert_eq!(widths[1], widths[2]);
  }

  #[test]
  fn the_box_is_measured_in_characters_not_bytes() {
    // A unicode host would otherwise draw a box wider than its contents.
    let mut out = String::new();
    frame(&mut out, &plain(), "http://привет:8010/");

    let lines: Vec<usize> = out
      .lines()
      .filter(|l| !l.trim().is_empty())
      .map(|l| l.chars().count())
      .collect();
    assert_eq!(lines[0], lines[1]);
    assert_eq!(lines[1], lines[2]);
  }

  #[test]
  fn a_row_pads_its_label_to_a_fixed_column() {
    let mut out = String::new();
    row(&mut out, &plain(), "root", "~/site");
    row(&mut out, &plain(), "not found", "built-in page");

    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "  root         ~/site");
    assert_eq!(lines[1], "  not found    built-in page");
    // The values start in the same column, which is the point of the padding.
    assert_eq!(lines[0].find("~/site"), lines[1].find("built-in page"));
  }

  #[test]
  fn printing_the_banner_does_not_panic() {
    // The whole block, over a real directory: the only way to reach `print` is
    // to call it, and a panic here would kill startup before the first request.
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("index.html"), "hi").unwrap();

    for (spa, not_found, ext, markdown, quiet) in [
      (None, None, false, false, false),
      (Some("index.html"), Some("404.html"), true, false, true),
      (Some("gone.html"), Some("gone.html"), false, true, true),
      (None, None, false, true, false),
    ] {
      let config = Config {
        root: dir.path().to_path_buf(),
        spa: spa.map(|p| dir.path().join(p)),
        not_found: not_found.map(|p| dir.path().join(p)),
        ext,
        markdown,
      };
      print(&config, "127.0.0.1:8010".parse().unwrap(), quiet);
    }
  }
}
