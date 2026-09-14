use std::convert::Infallible;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use damask::Component;
use hyper::body::Incoming;
use hyper::header::{ACCEPT, CONTENT_TYPE, HOST, LOCATION};
use hyper::{Method, Request, Response, StatusCode};

use crate::body::{self, Body};
use crate::config::Config;
use crate::logging;
use crate::pages::{document::Document, listing, not_found::NotFound};
use crate::{file, path};

pub async fn handle(
  req: Request<Incoming>,
  config: Arc<Config>,
) -> Result<Response<Body>, Infallible> {
  let started = Instant::now();

  let response = match route(&req, &config).await {
    Ok(response) => response,
    Err(err) => {
      log::error!("{} {}: {err}", req.method(), req.uri().path());
      text(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
    }
  };

  let size = response
    .headers()
    .get(hyper::header::CONTENT_LENGTH)
    .and_then(|value| value.to_str().ok())
    .and_then(|value| value.parse().ok());

  logging::request(
    req.method().as_str(),
    req.uri().path(),
    response.status().as_u16(),
    size,
    started.elapsed(),
  );

  Ok(response)
}

async fn route(req: &Request<Incoming>, config: &Config) -> io::Result<Response<Body>> {
  if !matches!(*req.method(), Method::GET | Method::HEAD) {
    return Ok(text(StatusCode::METHOD_NOT_ALLOWED, "method not allowed"));
  }

  let url_path = req.uri().path();
  let Some(target) = path::resolve(&config.root, url_path) else {
    return miss(req, config).await;
  };
  if !path::is_within(&config.root, &target) {
    return miss(req, config).await;
  }

  // Clean URLs: `/about.html` is the same page as `/about`, so send visitors
  // to the canonical one instead of serving both. With `-m` a document has the
  // same claim to one address as a page does.
  if !config.ext
    && let Some(ext) = page_suffix(url_path, config)
    && is_file(&target).await
  {
    return Ok(redirect(req, strip_suffix(url_path, ext)));
  }

  if is_file(&target).await {
    if config.markdown && is_markdown(&target) {
      return document(&target, url_path, config, req).await;
    }
    return file::send(&target, req, StatusCode::OK).await;
  }

  if is_dir(&target).await {
    // Relative links inside the page only resolve correctly under a trailing slash.
    if !url_path.ends_with('/') {
      return Ok(redirect(req, format!("{url_path}/")));
    }
    let index = target.join("index.html");
    if is_file(&index).await {
      return file::send(&index, req, StatusCode::OK).await;
    }
    // A folder that carries its own document shows it rather than a file list —
    // pointing serv at a project should open its README, the way a repository does.
    if config.markdown {
      for name in ["index.md", "README.md"] {
        let doc = target.join(name);
        if is_file(&doc).await {
          return document(&doc, url_path, config, req).await;
        }
      }
    }
    let index = listing::read(&target, url_path, &config.root).await?;
    return Ok(html(StatusCode::OK, req.method(), index.render()));
  }

  if !config.ext {
    let candidate = with_suffix(&target, ".html");
    if is_file(&candidate).await {
      return file::send(&candidate, req, StatusCode::OK).await;
    }
    if config.markdown {
      let candidate = with_suffix(&target, ".md");
      if is_file(&candidate).await {
        return document(&candidate, url_path, config, req).await;
      }
    }
  }

  miss(req, config).await
}

/// Nothing on disk matched. A single-page app answers with its shell; anything
/// else gets the not-found page.
async fn miss(req: &Request<Incoming>, config: &Config) -> io::Result<Response<Body>> {
  if let Some(spa) = &config.spa
    && wants_html(req)
    && is_file(spa).await
  {
    return file::send(spa, req, StatusCode::OK).await;
  }
  not_found(req, config).await
}

/// Only navigations get the SPA shell — a missing script should stay a 404
/// rather than turn into an HTML file with a confusing MIME type. Browsers ask
/// for `text/html` when navigating; a bare path with no extension is treated as
/// a route too, so `curl /some/route` behaves the way you would expect.
fn wants_html(req: &Request<Incoming>) -> bool {
  let accepts_html = req
    .headers()
    .get(ACCEPT)
    .and_then(|value| value.to_str().ok())
    .is_some_and(|accept| accept.contains("text/html"));

  accepts_html || !last_segment(req.uri().path()).contains('.')
}

fn last_segment(url_path: &str) -> &str {
  url_path.rsplit('/').next().unwrap_or("")
}

/// The extension this URL would be redirected out of, if any. `.md` counts only
/// when serv is rendering markdown; otherwise it is an ordinary file to hand over.
fn page_suffix(url_path: &str, config: &Config) -> Option<&'static str> {
  if url_path.ends_with(".html") {
    Some(".html")
  } else if config.markdown && url_path.ends_with(".md") {
    Some(".md")
  } else {
    None
  }
}

fn strip_suffix(url_path: &str, ext: &str) -> String {
  let stem = url_path.strip_suffix(ext).unwrap_or(url_path);
  match stem.rsplit_once('/') {
    // `/docs/index.html` is `/docs/`, and `/index.html` is `/`.
    Some((parent, "index")) => format!("{parent}/"),
    _ => stem.to_string(),
  }
}

fn with_suffix(path: &std::path::Path, ext: &str) -> PathBuf {
  let mut name = OsString::from(path);
  name.push(ext);
  PathBuf::from(name)
}

fn is_markdown(path: &std::path::Path) -> bool {
  path
    .extension()
    .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

/// Read a markdown file and set it on serv's own sheet. Like every other file
/// serv hands out, it is read fresh — editing it shows up on the next reload.
async fn document(
  path: &std::path::Path,
  url_path: &str,
  config: &Config,
  req: &Request<Incoming>,
) -> io::Result<Response<Body>> {
  let source = tokio::fs::read_to_string(path).await?;
  let name = path
    .file_name()
    .map(|n| n.to_string_lossy().into_owned())
    .unwrap_or_default();
  let page = Document::new(&source, &name, url_path, &config.root);

  Ok(html(StatusCode::OK, req.method(), page.render()))
}

async fn is_file(path: &std::path::Path) -> bool {
  tokio::fs::metadata(path).await.is_ok_and(|m| m.is_file())
}

async fn is_dir(path: &std::path::Path) -> bool {
  tokio::fs::metadata(path).await.is_ok_and(|m| m.is_dir())
}

fn redirect(req: &Request<Incoming>, location: String) -> Response<Body> {
  let location = match req.uri().query() {
    Some(query) => format!("{location}?{query}"),
    None => location,
  };
  Response::builder()
    .status(StatusCode::MOVED_PERMANENTLY)
    .header(LOCATION, location)
    .body(body::empty())
    .expect("valid response")
}

/// The not-found page. A `--not-found` file is read on every miss, so editing it
/// shows up on the next reload just like any other file serv hands out.
async fn not_found(req: &Request<Incoming>, config: &Config) -> io::Result<Response<Body>> {
  if let Some(page) = &config.not_found
    && is_file(page).await
  {
    return file::send(page, req, StatusCode::NOT_FOUND).await;
  }

  let host = req
    .headers()
    .get(HOST)
    .and_then(|value| value.to_str().ok())
    .unwrap_or("localhost");
  let page = NotFound::new(req.uri().path(), host);

  Ok(html(StatusCode::NOT_FOUND, req.method(), page.render()))
}

fn html(status: StatusCode, method: &Method, markup: String) -> Response<Body> {
  let len = markup.len();
  let body = if method == Method::HEAD {
    body::empty()
  } else {
    body::full(markup)
  };
  Response::builder()
    .status(status)
    .header(CONTENT_TYPE, "text/html; charset=utf-8")
    .header(hyper::header::CONTENT_LENGTH, len)
    .body(body)
    .expect("valid response")
}

fn text(status: StatusCode, message: &'static str) -> Response<Body> {
  Response::builder()
    .status(status)
    .header(CONTENT_TYPE, "text/plain; charset=utf-8")
    .body(body::full(message))
    .expect("valid response")
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::testkit::{Req, Site};

  const PAGE: &str = "<!doctype html><title>about</title>";

  fn site() -> Site {
    Site::new()
  }

  // ---- clean URLs -------------------------------------------------------

  #[tokio::test]
  async fn serves_a_page_without_its_extension() {
    let site = site().file("about.html", PAGE);
    let reply = site.get("/about").await;

    assert_eq!(reply.status, 200);
    assert_eq!(
      reply.header("content-type"),
      Some("text/html; charset=utf-8")
    );
    assert_eq!(reply.text(), PAGE);
  }

  #[tokio::test]
  async fn serves_a_nested_page_without_its_extension() {
    let site = site().file("docs/guide.html", PAGE);

    assert_eq!(site.get("/docs/guide").await.status, 200);
  }

  #[tokio::test]
  async fn redirects_the_extension_to_the_clean_url() {
    let site = site().file("about.html", PAGE);
    let reply = site.get("/about.html").await;

    assert_eq!(reply.status, 301);
    assert_eq!(reply.header("location"), Some("/about"));
    assert!(reply.body.is_empty());
  }

  #[tokio::test]
  async fn a_nested_index_redirects_to_its_directory() {
    let site = site().file("docs/index.html", PAGE);
    let reply = site.get("/docs/index.html").await;

    assert_eq!(reply.status, 301);
    assert_eq!(reply.header("location"), Some("/docs/"));
  }

  #[tokio::test]
  async fn the_root_index_redirects_to_the_root() {
    let site = site().file("index.html", PAGE);
    let reply = site.get("/index.html").await;

    assert_eq!(reply.status, 301);
    assert_eq!(reply.header("location"), Some("/"));
  }

  #[tokio::test]
  async fn a_redirect_carries_the_query_string_over() {
    // Losing `?ref=newsletter` on a redirect loses the reason the visitor came.
    let site = site().file("about.html", PAGE);
    let reply = site.get("/about.html?ref=newsletter&x=1").await;

    assert_eq!(reply.status, 301);
    assert_eq!(reply.header("location"), Some("/about?ref=newsletter&x=1"));
  }

  #[tokio::test]
  async fn an_empty_query_string_still_survives() {
    let site = site().file("about.html", PAGE);
    let reply = site.get("/about.html?").await;

    assert_eq!(reply.header("location"), Some("/about?"));
  }

  #[tokio::test]
  async fn only_a_real_file_is_redirected() {
    // `/gone.html` has nothing behind it, so it is a miss rather than a
    // redirect to a `/gone` that is equally absent.
    let site = site();
    let reply = site
      .send(Req::get("/gone.html").header("accept", "*/*"))
      .await;

    assert_eq!(reply.status, 404);
  }

  #[tokio::test]
  async fn a_trailing_slash_also_finds_the_page() {
    // Current behaviour: `/about/` resolves to `about`, finds nothing, and the
    // `.html` suffix is tried there too — so the page answers on `/about/` as
    // well as `/about`, without a redirect between them.
    let site = site().file("about.html", PAGE);
    let reply = site.get("/about/").await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), PAGE);
  }

  // ---- directories ------------------------------------------------------

  #[tokio::test]
  async fn a_directory_with_an_index_serves_it() {
    let site = site().file("docs/index.html", PAGE);
    let reply = site.get("/docs/").await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), PAGE);
  }

  #[tokio::test]
  async fn the_root_serves_its_index() {
    let site = site().file("index.html", PAGE);
    let reply = site.get("/").await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), PAGE);
  }

  #[tokio::test]
  async fn a_directory_without_a_trailing_slash_is_redirected_to_one() {
    // Relative links inside the page only resolve correctly under the slash.
    let site = site().folder("docs");
    let reply = site.get("/docs").await;

    assert_eq!(reply.status, 301);
    assert_eq!(reply.header("location"), Some("/docs/"));
  }

  #[tokio::test]
  async fn the_trailing_slash_redirect_keeps_the_query_too() {
    let site = site().folder("docs");
    let reply = site.get("/docs?page=2").await;

    assert_eq!(reply.header("location"), Some("/docs/?page=2"));
  }

  #[tokio::test]
  async fn a_directory_without_an_index_gets_a_listing() {
    let site = site().file("docs/a.txt", "a").file("docs/b.txt", "b");
    let reply = site.get("/docs/").await;

    assert_eq!(reply.status, 200);
    assert_eq!(
      reply.header("content-type"),
      Some("text/html; charset=utf-8")
    );
    let html = reply.text();
    assert!(html.contains("a.txt"));
    assert!(html.contains("b.txt"));
    assert_eq!(reply.content_length(), Some(reply.body.len() as u64));
  }

  #[tokio::test]
  async fn the_root_gets_a_listing_when_there_is_no_index() {
    let site = site().file("notes.md", "hi");
    let reply = site.get("/").await;

    assert_eq!(reply.status, 200);
    assert!(reply.text().contains("notes.md"));
  }

  #[tokio::test]
  async fn a_directory_named_like_a_page_is_still_a_directory() {
    // `about.html` as a folder is perverse, but the `.html` redirect only fires
    // for a file, so this lands on the trailing-slash redirect instead.
    let site = site().folder("about.html");
    let reply = site.get("/about.html").await;

    assert_eq!(reply.status, 301);
    assert_eq!(reply.header("location"), Some("/about.html/"));
  }

  // ---- -e / literal URLs ------------------------------------------------

  #[tokio::test]
  async fn ext_mode_requires_the_extension() {
    let site = site().ext().file("about.html", PAGE);

    let literal = site.get("/about.html").await;
    assert_eq!(literal.status, 200);
    assert_eq!(literal.text(), PAGE);

    let clean = site.send(Req::get("/about").header("accept", "*/*")).await;
    assert_eq!(clean.status, 404);
  }

  #[tokio::test]
  async fn ext_mode_leaves_the_root_index_alone() {
    let site = site().ext().file("index.html", PAGE);

    assert_eq!(site.get("/index.html").await.status, 200);
    assert_eq!(site.get("/").await.status, 200);
  }

  #[tokio::test]
  async fn ext_mode_still_redirects_a_directory_and_still_lists_it() {
    // `-e` turns off the `.html` handling, nothing else.
    let site = site().ext().file("docs/a.txt", "a");

    assert_eq!(site.get("/docs").await.header("location"), Some("/docs/"));
    assert!(site.get("/docs/").await.text().contains("a.txt"));
  }

  // ---- single-page apps -------------------------------------------------

  #[tokio::test]
  async fn a_navigation_falls_back_to_the_shell() {
    let site = site().spa("index.html").file("index.html", PAGE);
    let reply = site
      .send(Req::get("/users/42").header("accept", "text/html,application/xhtml+xml"))
      .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), PAGE);
  }

  #[tokio::test]
  async fn a_bare_route_is_treated_as_a_navigation() {
    // No Accept header at all: a path whose last segment has no dot is a route,
    // so `curl /some/route` behaves the way you would expect.
    let site = site().spa("index.html").file("index.html", PAGE);
    let reply = site
      .send(Req::get("/some/route").header("accept", "*/*"))
      .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), PAGE);
  }

  #[tokio::test]
  async fn a_missing_script_stays_a_404() {
    // The failure mode that costs an afternoon: a 200 of HTML served as
    // JavaScript.
    let site = site().spa("index.html").file("index.html", PAGE);
    let reply = site.send(Req::get("/app.js").header("accept", "*/*")).await;

    assert_eq!(reply.status, 404);
    assert!(!reply.text().contains("<title>about"));
  }

  #[tokio::test]
  async fn a_missing_stylesheet_stays_a_404_too() {
    let site = site().spa("index.html").file("index.html", PAGE);
    let reply = site
      .send(Req::get("/app.css").header("accept", "text/css,*/*;q=0.1"))
      .await;

    assert_eq!(reply.status, 404);
  }

  #[tokio::test]
  async fn a_dotted_path_asked_for_as_html_does_get_the_shell() {
    // Accept is the stronger signal of the two: a browser navigating to
    // `/report.2024` sends `text/html`, and that is a route, not an asset.
    let site = site().spa("index.html").file("index.html", PAGE);
    let reply = site
      .send(Req::get("/report.2024").header("accept", "text/html"))
      .await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), PAGE);
  }

  #[tokio::test]
  async fn a_shell_that_is_not_on_disk_leaves_the_404_in_place() {
    let site = site().spa("missing.html");
    let reply = site.get("/route").await;

    assert_eq!(reply.status, 404);
  }

  #[tokio::test]
  async fn the_shell_never_shadows_a_real_file() {
    let site = site()
      .spa("index.html")
      .file("index.html", PAGE)
      .file("robots.txt", "User-agent: *");
    let reply = site.get("/robots.txt").await;

    assert_eq!(reply.text(), "User-agent: *");
  }

  #[tokio::test]
  async fn the_shell_is_re_read_on_every_request() {
    // Nothing is cached in the server, so editing the shell shows up on the
    // next reload like any other file.
    let site = site().spa("index.html").file("index.html", "first");
    assert_eq!(site.get("/route").await.text(), "first");

    std::fs::write(site.path().join("index.html"), "second").unwrap();
    assert_eq!(site.get("/route").await.text(), "second");
  }

  #[tokio::test]
  async fn the_shell_answers_with_200_not_404() {
    let site = site().spa("index.html").file("index.html", PAGE);

    assert_eq!(site.get("/route").await.status, 200);
  }

  // ---- misses -----------------------------------------------------------

  #[tokio::test]
  async fn the_built_in_404_names_the_path_and_the_host() {
    let site = site();
    let reply = site
      .send(Req::get("/missing/page").header("host", "localhost:8010"))
      .await;

    assert_eq!(reply.status, 404);
    assert_eq!(
      reply.header("content-type"),
      Some("text/html; charset=utf-8")
    );
    let html = reply.text();
    assert!(html.contains("/missing/page"));
    assert!(html.contains("localhost:8010"));
  }

  #[tokio::test]
  async fn a_custom_not_found_page_is_served_with_a_404_status() {
    let site = site()
      .not_found("404.html")
      .file("404.html", "<h1>gone</h1>");
    let reply = site.get("/missing").await;

    assert_eq!(reply.status, 404);
    assert_eq!(reply.text(), "<h1>gone</h1>");
    assert_eq!(
      reply.header("content-type"),
      Some("text/html; charset=utf-8")
    );
  }

  #[tokio::test]
  async fn a_custom_not_found_page_is_re_read_on_every_miss() {
    let site = site().not_found("404.html").file("404.html", "first");
    assert_eq!(site.get("/missing").await.text(), "first");

    std::fs::write(site.path().join("404.html"), "second").unwrap();
    assert_eq!(site.get("/missing").await.text(), "second");
  }

  #[tokio::test]
  async fn a_custom_not_found_page_that_is_gone_falls_back_to_the_built_in_one() {
    let site = site().not_found("404.html");
    let reply = site.get("/missing").await;

    assert_eq!(reply.status, 404);
    assert!(reply.text().contains("/missing"));
  }

  #[tokio::test]
  async fn the_shell_wins_over_the_not_found_page_for_a_navigation() {
    let site = site()
      .spa("index.html")
      .not_found("404.html")
      .file("index.html", PAGE)
      .file("404.html", "gone");

    assert_eq!(site.get("/route").await.status, 200);
    // ...and the not-found page still answers for an asset.
    let asset = site.send(Req::get("/app.js").header("accept", "*/*")).await;
    assert_eq!(asset.status, 404);
    assert_eq!(asset.text(), "gone");
  }

  // ---- traversal --------------------------------------------------------

  #[tokio::test]
  async fn climbing_out_of_the_root_is_a_miss() {
    let site = site().file("index.html", PAGE);

    for path in ["/../secret", "/a/../../secret", "/%2e%2e/secret"] {
      let reply = site.send(Req::get(path).header("accept", "*/*")).await;
      assert_eq!(reply.status, 404, "{path}");
    }
  }

  #[tokio::test]
  async fn a_smuggled_separator_is_a_miss() {
    let site = site().file("index.html", PAGE);

    for path in ["/a%2f..%2f..%2fsecret", "/a%5c..%5csecret"] {
      let reply = site.send(Req::get(path).header("accept", "*/*")).await;
      assert_eq!(reply.status, 404, "{path}");
    }
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn a_symlink_pointing_out_of_the_root_is_a_miss() {
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "shh").unwrap();

    let site = site();
    std::os::unix::fs::symlink(
      outside.path().join("secret.txt"),
      site.path().join("escape.txt"),
    )
    .unwrap();

    let reply = site
      .send(Req::get("/escape.txt").header("accept", "*/*"))
      .await;
    assert_eq!(reply.status, 404);
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn a_symlink_staying_inside_the_root_is_followed() {
    let site = site().file("real.txt", "hi");
    std::os::unix::fs::symlink(site.path().join("real.txt"), site.path().join("alias.txt"))
      .unwrap();

    let reply = site.get("/alias.txt").await;
    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), "hi");
  }

  // ---- methods ----------------------------------------------------------

  #[tokio::test]
  async fn anything_but_get_and_head_is_refused() {
    let site = site().file("index.html", PAGE);

    for method in ["POST", "PUT", "DELETE", "PATCH", "OPTIONS"] {
      let reply = site.send(Req::new(method, "/")).await;
      assert_eq!(reply.status, 405, "{method}");
      assert_eq!(
        reply.header("content-type"),
        Some("text/plain; charset=utf-8"),
        "{method}"
      );
      assert_eq!(reply.text(), "method not allowed", "{method}");
    }
  }

  #[tokio::test]
  async fn a_refused_method_is_refused_before_the_path_is_looked_at() {
    // The method check comes first, so a POST to a traversal is a 405 and never
    // touches the filesystem.
    let site = site();
    let reply = site.send(Req::new("POST", "/../secret")).await;

    assert_eq!(reply.status, 405);
  }

  #[tokio::test]
  async fn head_declares_the_length_and_sends_no_body() {
    let site = site().file("about.html", PAGE);
    let reply = site.send(Req::head("/about")).await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.content_length(), Some(PAGE.len() as u64));
    assert!(reply.body.is_empty());
  }

  #[tokio::test]
  async fn head_on_a_listing_sends_no_body() {
    let site = site().file("docs/a.txt", "a");
    let reply = site.send(Req::head("/docs/")).await;

    assert_eq!(reply.status, 200);
    assert!(reply.body.is_empty());
    assert!(reply.content_length().unwrap() > 0);
  }

  #[tokio::test]
  async fn head_on_a_miss_sends_no_body() {
    let site = site();
    let reply = site.send(Req::head("/missing")).await;

    assert_eq!(reply.status, 404);
    assert!(reply.body.is_empty());
    assert!(reply.content_length().unwrap() > 0);
  }

  #[tokio::test]
  async fn head_still_redirects() {
    let site = site().file("about.html", PAGE);
    let reply = site.send(Req::head("/about.html")).await;

    assert_eq!(reply.status, 301);
    assert_eq!(reply.header("location"), Some("/about"));
  }

  // ---- names on the wire ------------------------------------------------

  #[tokio::test]
  async fn serves_a_name_with_a_space_in_it() {
    let site = site().file("my file.txt", "hi");
    let reply = site.get("/my%20file.txt").await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), "hi");
  }

  #[tokio::test]
  async fn serves_a_unicode_name() {
    let site = site().file("привет.txt", "hi");
    let reply = site.get("/%D0%BF%D1%80%D0%B8%D0%B2%D0%B5%D1%82.txt").await;

    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), "hi");
  }

  #[tokio::test]
  async fn a_listing_links_to_names_it_can_serve_back() {
    // The listing's encoding and the request path's decoding have to agree, or
    // every awkward name in the folder is a broken link.
    let site = site().file("docs/my file #1.txt", "hi");
    let html = site.get("/docs/").await.text();

    assert!(html.contains("my%20file%20%231.txt"));
    let reply = site.get("/docs/my%20file%20%231.txt").await;
    assert_eq!(reply.text(), "hi");
  }

  // ---- errors -----------------------------------------------------------

  #[cfg(unix)]
  #[tokio::test]
  async fn a_directory_that_cannot_be_read_is_a_500() {
    use std::os::unix::fs::PermissionsExt;

    let site = site().folder("locked");
    let locked = site.path().join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // root reads it regardless, and then there is nothing to test.
    if std::fs::read_dir(&locked).is_ok() {
      return;
    }

    let reply = site.get("/locked/").await;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(reply.status, 500);
    assert_eq!(reply.text(), "internal server error");
  }

  // ---- the helpers behind the routing ------------------------------------

  #[test]
  fn strips_the_extension_and_folds_an_index_into_its_directory() {
    assert_eq!(strip_suffix("/about.html", ".html"), "/about");
    assert_eq!(strip_suffix("/docs/guide.html", ".html"), "/docs/guide");
    assert_eq!(strip_suffix("/index.html", ".html"), "/");
    assert_eq!(strip_suffix("/docs/index.html", ".html"), "/docs/");
    assert_eq!(strip_suffix("/a/b/index.html", ".html"), "/a/b/");
  }

  #[test]
  fn strips_a_markdown_extension_the_same_way() {
    assert_eq!(strip_suffix("/about.md", ".md"), "/about");
    assert_eq!(strip_suffix("/docs/index.md", ".md"), "/docs/");
    assert_eq!(strip_suffix("/index.md", ".md"), "/");
  }

  #[test]
  fn stripping_the_extension_removes_exactly_one() {
    // `strip_suffix` where this used to be a `trim_end_matches`, which removed
    // every trailing `.html` and sent `page.html.html` to `/page`.
    assert_eq!(strip_suffix("/page.html.html", ".html"), "/page.html");
    assert_eq!(strip_suffix("/notes.md.md", ".md"), "/notes.md");
  }

  #[test]
  fn the_last_segment_is_what_decides_a_navigation() {
    assert_eq!(last_segment("/a/b/c.js"), "c.js");
    assert_eq!(last_segment("/a/b/"), "");
    assert_eq!(last_segment("/"), "");
    assert_eq!(last_segment("plain"), "plain");
  }

  #[test]
  fn the_suffix_is_appended_not_substituted() {
    assert_eq!(
      with_suffix(std::path::Path::new("/srv/about"), ".html"),
      PathBuf::from("/srv/about.html")
    );
    assert_eq!(
      with_suffix(std::path::Path::new("/srv/a.b"), ".html"),
      PathBuf::from("/srv/a.b.html")
    );
    assert_eq!(
      with_suffix(std::path::Path::new("/srv/about"), ".md"),
      PathBuf::from("/srv/about.md")
    );
  }

  #[tokio::test]
  async fn tells_a_file_from_a_directory_from_nothing() {
    let site = site().file("a.txt", "x").folder("docs");

    assert!(is_file(&site.path().join("a.txt")).await);
    assert!(!is_file(&site.path().join("docs")).await);
    assert!(!is_file(&site.path().join("nope")).await);

    assert!(is_dir(&site.path().join("docs")).await);
    assert!(!is_dir(&site.path().join("a.txt")).await);
    assert!(!is_dir(&site.path().join("nope")).await);
  }
}

#[cfg(test)]
mod markdown_tests {
  use crate::testkit::{Req, Site};

  const DOC: &str = "# Reading\n\nsome prose\n";

  fn site() -> Site {
    Site::new().markdown()
  }

  // ---- rendering --------------------------------------------------------

  #[tokio::test]
  async fn renders_a_markdown_file_as_a_page() {
    let reply = site().file("about.md", DOC).get("/about").await;

    assert_eq!(reply.status, 200);
    assert_eq!(
      reply.header("content-type"),
      Some("text/html; charset=utf-8")
    );
    assert!(reply.text().contains("<h1 id=\"reading\">Reading</h1>"));
    assert!(reply.text().contains("<p>some prose</p>"));
  }

  #[tokio::test]
  async fn without_the_flag_markdown_is_served_as_it_is_on_disk() {
    let reply = Site::new().file("about.md", DOC).get("/about.md").await;

    assert_eq!(reply.status, 200);
    assert_eq!(
      reply.header("content-type"),
      Some("text/markdown; charset=utf-8")
    );
    assert_eq!(reply.text(), DOC);
  }

  #[tokio::test]
  async fn without_the_flag_a_clean_url_does_not_find_markdown() {
    assert_eq!(
      Site::new().file("about.md", DOC).get("/about").await.status,
      404
    );
  }

  // ---- one address per page ---------------------------------------------

  #[tokio::test]
  async fn redirects_the_extension_to_the_clean_url() {
    let reply = site().file("about.md", DOC).get("/about.md").await;

    assert_eq!(reply.status, 301);
    assert_eq!(reply.header("location"), Some("/about"));
  }

  #[tokio::test]
  async fn a_nested_index_redirects_to_its_directory() {
    let reply = site()
      .file("docs/index.md", DOC)
      .get("/docs/index.md")
      .await;

    assert_eq!(reply.status, 301);
    assert_eq!(reply.header("location"), Some("/docs/"));
  }

  #[tokio::test]
  async fn the_root_index_redirects_to_the_root() {
    let reply = site().file("index.md", DOC).get("/index.md").await;

    assert_eq!(reply.status, 301);
    assert_eq!(reply.header("location"), Some("/"));
  }

  #[tokio::test]
  async fn a_redirect_keeps_the_query_string() {
    let reply = site().file("about.md", DOC).get("/about.md?x=1").await;

    assert_eq!(reply.header("location"), Some("/about?x=1"));
  }

  // ---- html wins --------------------------------------------------------

  #[tokio::test]
  async fn html_beats_markdown_at_the_same_clean_url() {
    let reply = site()
      .file("about.html", "<p>html</p>")
      .file("about.md", DOC)
      .get("/about")
      .await;

    assert_eq!(reply.text(), "<p>html</p>");
  }

  #[tokio::test]
  async fn an_html_index_beats_a_markdown_one() {
    let reply = site()
      .file("docs/index.html", "<p>html</p>")
      .file("docs/index.md", DOC)
      .get("/docs/")
      .await;

    assert_eq!(reply.text(), "<p>html</p>");
  }

  // ---- a folder's own page ----------------------------------------------

  #[tokio::test]
  async fn a_folder_with_an_index_md_serves_it_instead_of_a_listing() {
    let reply = site().file("docs/index.md", DOC).get("/docs/").await;

    assert_eq!(reply.status, 200);
    assert!(reply.text().contains("some prose"));
    assert!(!reply.text().contains("Index of"));
  }

  #[tokio::test]
  async fn a_folder_falls_back_to_its_readme() {
    let reply = site().file("docs/README.md", DOC).get("/docs/").await;

    assert_eq!(reply.status, 200);
    assert!(reply.text().contains("some prose"));
  }

  #[tokio::test]
  async fn an_index_md_beats_a_readme() {
    let reply = site()
      .file("docs/index.md", "# Index\n")
      .file("docs/README.md", "# Readme\n")
      .get("/docs/")
      .await;

    assert!(reply.text().contains("Index"), "{}", reply.text());
    assert!(!reply.text().contains("Readme"));
  }

  #[tokio::test]
  async fn a_folder_with_no_document_still_gets_its_listing() {
    let reply = site().file("docs/note.txt", "x").get("/docs/").await;

    assert!(reply.text().contains("Index of"), "{}", reply.text());
  }

  #[tokio::test]
  async fn without_the_flag_a_readme_does_not_replace_the_listing() {
    let reply = Site::new().file("docs/README.md", DOC).get("/docs/").await;

    assert!(reply.text().contains("Index of"), "{}", reply.text());
  }

  // ---- alongside the other flags ----------------------------------------

  #[tokio::test]
  async fn ext_mode_serves_the_extension_and_still_renders() {
    let site = Site::new().markdown().ext().file("about.md", DOC);
    let reply = site.get("/about.md").await;

    assert_eq!(reply.status, 200);
    assert!(reply.text().contains("some prose"));
  }

  #[tokio::test]
  async fn ext_mode_does_not_strip_the_extension() {
    let site = Site::new().markdown().ext().file("about.md", DOC);

    assert_eq!(site.get("/about").await.status, 404);
  }

  #[tokio::test]
  async fn ext_mode_still_lets_a_folder_serve_its_readme() {
    // The fallback is about which file a folder means, not about extensions.
    let site = Site::new().markdown().ext().file("docs/README.md", DOC);

    assert!(site.get("/docs/").await.text().contains("some prose"));
  }

  #[tokio::test]
  async fn a_missing_document_is_still_a_404() {
    let reply = site().file("about.md", DOC).get("/missing").await;

    assert_eq!(reply.status, 404);
  }

  #[tokio::test]
  async fn a_head_request_carries_no_body() {
    let site = site().file("about.md", DOC);
    let reply = site.send(Req::head("/about")).await;

    assert_eq!(reply.status, 200);
    assert!(reply.body.is_empty());
    assert!(reply.content_length().is_some_and(|len| len > 0));
  }

  #[tokio::test]
  async fn the_spa_shell_still_wins_for_an_unmatched_route() {
    let site = Site::new()
      .markdown()
      .spa("app.html")
      .file("app.html", "<p>shell</p>")
      .file("about.md", DOC);

    assert_eq!(site.get("/some/route").await.text(), "<p>shell</p>");
    // A document that exists is still the document, not the shell.
    assert!(site.get("/about").await.text().contains("some prose"));
  }
}
