//! Request logging, in the shape a dev server wants: one line per request, no
//! timestamps, no target prefixes — just what was asked for and what came back.

use std::io::Write;
use std::time::Duration;

use log::LevelFilter;

use crate::style;

/// The target access lines are logged under, so `--quiet` can silence exactly
/// those and leave warnings and errors alone.
pub const ACCESS: &str = "serv::access";

pub fn init(quiet: bool) {
  let mut builder =
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));

  // The closure writes the whole line, newline included.
  builder.format_suffix("");
  builder.format(|out, record| {
    if record.target() == ACCESS {
      writeln!(out, "{}", record.args())
    } else {
      let level = record.level().as_str().to_lowercase();
      writeln!(out, "{level}: {}", record.args())
    }
  });

  if quiet {
    builder.filter_module(ACCESS, LevelFilter::Off);
  }
  builder.init();
}

pub fn request(method: &str, path: &str, status: u16, size: Option<u64>, took: Duration) {
  let meta = match size {
    Some(size) => format!("{}  {}", human_size(size), human_time(took)),
    None => human_time(took),
  };

  log::info!(
    target: ACCESS,
    "{} {method:<4} {path:<38} {}",
    status_badge(status),
    style::paint(style::stderr(), "2", &meta),
  );
}

/// Colour by class, the way every access log worth reading does.
fn status_badge(status: u16) -> String {
  let colour = match status {
    200..=299 => "32",
    300..=399 => "36",
    400..=499 => "33",
    _ => "31",
  };
  style::paint(style::stderr(), colour, &status.to_string())
}

fn human_size(bytes: u64) -> String {
  crate::pages::human_size(bytes)
}

fn human_time(took: Duration) -> String {
  let millis = took.as_secs_f64() * 1000.0;
  if millis < 1.0 {
    format!("{:.0} µs", took.as_micros())
  } else {
    format!("{millis:.1} ms")
  }
}
