//! The two pages serv draws itself: a directory index and a 404.
//!
//! Both are Damask components — the template is compiled into the binary, so a
//! page that reads a field the struct does not have is a build failure.

pub mod listing;

/// One stylesheet, shared by both pages and inlined into each response. A dev
/// server has to work with the network unplugged, so nothing is fetched.
pub const STYLE: &str = include_str!("../../assets/style.css");

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
