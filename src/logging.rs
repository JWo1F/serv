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

#[cfg(test)]
mod tests {
  use super::*;

  /// What `status_badge` and `request` paint with depends on whether stderr is a
  /// terminal, which the test runner decides. Expectations are built the same
  /// way so both cases hold.
  fn dim(text: &str) -> String {
    style::paint(style::stderr(), "2", text)
  }

  #[test]
  fn access_lines_have_their_own_target() {
    // `--quiet` silences exactly this target and leaves warnings alone.
    assert_eq!(ACCESS, "serv::access");
  }

  #[test]
  fn colours_a_status_by_its_class() {
    for (status, colour) in [
      (200u16, "32"),
      (204, "32"),
      (299, "32"),
      (301, "36"),
      (304, "36"),
      (404, "33"),
      (405, "33"),
      (416, "33"),
      (500, "31"),
      (100, "31"),
    ] {
      assert_eq!(
        status_badge(status),
        style::paint(style::stderr(), colour, &status.to_string()),
        "status {status}"
      );
    }
  }

  #[test]
  fn a_badge_always_says_the_number() {
    for status in [200u16, 301, 404, 500] {
      assert!(status_badge(status).contains(&status.to_string()));
    }
  }

  #[test]
  fn sub_millisecond_timings_are_printed_in_microseconds() {
    assert_eq!(human_time(Duration::from_micros(0)), "0 µs");
    assert_eq!(human_time(Duration::from_micros(1)), "1 µs");
    assert_eq!(human_time(Duration::from_micros(999)), "999 µs");
  }

  #[test]
  fn a_millisecond_and_over_is_printed_in_milliseconds() {
    assert_eq!(human_time(Duration::from_millis(1)), "1.0 ms");
    assert_eq!(human_time(Duration::from_micros(1500)), "1.5 ms");
    assert_eq!(human_time(Duration::from_millis(1234)), "1234.0 ms");
    assert_eq!(human_time(Duration::from_secs(2)), "2000.0 ms");
  }

  #[test]
  fn sizes_are_the_same_ones_the_listing_prints() {
    assert_eq!(human_size(0), "0 B");
    assert_eq!(human_size(1023), "1023 B");
    assert_eq!(human_size(2048), "2.0 kB");
  }

  #[test]
  fn a_size_is_dropped_when_there_is_none_to_report() {
    // The `meta` half of a request line: size and timing, or timing alone when
    // the response had no Content-Length — a streamed 206, for instance.
    let with = format!(
      "{}  {}",
      human_size(2400),
      human_time(Duration::from_micros(600))
    );
    assert_eq!(with, "2.3 kB  600 µs");
    assert_eq!(human_time(Duration::from_micros(600)), "600 µs");
    assert!(dim(&with).contains("2.3 kB"));
  }

  #[test]
  fn logging_a_request_without_a_logger_installed_is_harmless() {
    // `init` is never called here — no global logger, so the macro drops the
    // record. The point is that building the line cannot panic.
    request(
      "GET",
      "/index.html",
      200,
      Some(2400),
      Duration::from_micros(600),
    );
    request("POST", "/", 405, None, Duration::from_millis(3));
  }
}
