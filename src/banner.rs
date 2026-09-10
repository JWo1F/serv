//! The block of text serv prints on startup.
//!
//! It answers the questions you would otherwise have to guess at: which folder,
//! which address, which page a miss lands on, and whether `/about` will find
//! `about.html`.

use std::fmt::Write as _;
use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::Path;

use crate::config::Config;
use crate::pages::human_size;

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
    &format!("brotli {} gzip", paint.on(DIM, "·")),
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
        Some(rest) if rest.is_empty() => "~".to_string(),
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
      on: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
    }
  }

  fn on(&self, code: &str, text: &str) -> String {
    if self.on {
      format!("\x1b[{code}m{text}\x1b[0m")
    } else {
      text.to_string()
    }
  }
}
