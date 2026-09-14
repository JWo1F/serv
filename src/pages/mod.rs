//! The two pages serv draws itself: a directory index and a 404.
//!
//! Both are Damask components — the template is compiled into the binary, so a
//! page that reads a field the struct does not have is a build failure.

pub mod document;
pub mod icons;
pub mod listing;
pub mod not_found;

/// One stylesheet, shared by both pages and inlined into each response. A dev
/// server has to work with the network unplugged, so nothing is fetched.
pub const STYLE: &str = include_str!("../../assets/style.css");

/// The mark, reduced to what survives a 16px browser tab: a typed stub and the
/// block cursor still sitting after it.
pub const FAVICON: &str = "data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><rect x='2.5' y='10.4' width='7.5' height='3.2' rx='1.6' fill='%23a3372a' opacity='.4'/><rect x='12.5' y='3.6' width='7' height='16.8' rx='1.8' fill='%23a3372a'/></svg>";

/// Bytes as a person would read them.
pub fn human_size(bytes: u64) -> String {
  const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
  let mut size = bytes as f64;
  let mut unit = 0;
  while size >= 1024.0 && unit < UNITS.len() - 1 {
    size /= 1024.0;
    unit += 1;
  }
  if unit == 0 {
    format!("{bytes} B")
  } else {
    format!("{size:.1} {}", UNITS[unit])
  }
}

/// `Thu, 04 Sep 2026 14:07:33 GMT` trimmed down to `04 Sep 2026 14:07`.
pub fn human_time(time: std::time::SystemTime) -> String {
  let stamp = httpdate::fmt_http_date(time);
  stamp.get(5..22).unwrap_or(&stamp).to_string()
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::time::{Duration, UNIX_EPOCH};

  #[test]
  fn bytes_below_a_kilobyte_are_printed_exactly() {
    assert_eq!(human_size(0), "0 B");
    assert_eq!(human_size(1), "1 B");
    assert_eq!(human_size(512), "512 B");
    assert_eq!(human_size(1023), "1023 B");
  }

  #[test]
  fn steps_up_a_unit_at_each_multiple_of_1024() {
    assert_eq!(human_size(1024), "1.0 kB");
    assert_eq!(human_size(1024 * 1024), "1.0 MB");
    assert_eq!(human_size(1024 * 1024 * 1024), "1.0 GB");
    assert_eq!(human_size(1024u64.pow(4)), "1.0 TB");
  }

  #[test]
  fn keeps_one_decimal_within_a_unit() {
    assert_eq!(human_size(1025), "1.0 kB");
    assert_eq!(human_size(1536), "1.5 kB");
    assert_eq!(human_size(1024 * 1024 - 1024), "1023.0 kB");
  }

  #[test]
  fn rounds_up_to_a_unit_it_has_not_reached() {
    // 1048575 is a byte short of a megabyte, and one decimal place cannot show
    // the difference — so the listing says `1024.0 kB` rather than `1.0 MB`.
    // Cosmetic, and only ever visible on the two bytes either side of a step.
    assert_eq!(human_size(1024 * 1024 - 1), "1024.0 kB");
  }

  #[test]
  fn stops_climbing_at_terabytes() {
    assert_eq!(human_size(1024u64.pow(5)), "1024.0 TB");
    assert_eq!(human_size(u64::MAX), "16777216.0 TB");
  }

  #[test]
  fn a_timestamp_loses_its_weekday_and_its_seconds() {
    assert_eq!(human_time(UNIX_EPOCH), "01 Jan 1970 00:00");
    assert_eq!(
      human_time(UNIX_EPOCH + Duration::from_secs(1_757_000_853)),
      "04 Sep 2025 15:47"
    );
  }

  #[test]
  fn a_timestamp_keeps_its_leading_zeroes() {
    // Fixed-width fields are what let the listing's date column line up.
    let stamp = human_time(UNIX_EPOCH + Duration::from_secs(60 * 60 * 5 + 60 * 7));
    assert_eq!(stamp, "01 Jan 1970 05:07");
    assert_eq!(stamp.len(), 17);
  }

  #[test]
  fn the_favicon_is_inlined_rather_than_fetched() {
    // A dev server has to draw its own pages with the network unplugged.
    assert!(FAVICON.starts_with("data:image/svg+xml,"));
    assert!(!STYLE.is_empty());
  }
}
