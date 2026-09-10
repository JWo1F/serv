//! The two pages serv draws itself: a directory index and a 404.
//!
//! Both are Damask components — the template is compiled into the binary, so a
//! page that reads a field the struct does not have is a build failure.

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
