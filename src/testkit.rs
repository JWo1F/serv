//! A site on disk, a real connection, and a hand-written client.
//!
//! `serve::handle` and `file::send` both take a `Request<Incoming>`, and an
//! `Incoming` body is only ever produced by hyper's own server — there is no
//! constructor for one. So rather than reshape the production code to be
//! reachable from a test, the tests drive it the way a browser does: bind a
//! loopback port, serve exactly one connection on it, and write the request
//! bytes by hand. hyper is compiled with the `server` feature only, hence the
//! client being fifty lines of `write_all` and `read_to_end` rather than a
//! dependency.
//!
//! The module is `#[cfg(test)]`, so none of it reaches the shipped binary.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::config::Config;
use crate::serve;

/// A temporary directory to serve, plus the `Config` main.rs would have built
/// from it.
pub struct Site {
  dir: TempDir,
  spa: Option<PathBuf>,
  not_found: Option<PathBuf>,
  ext: bool,
}

impl Default for Site {
  fn default() -> Self {
    Self::new()
  }
}

impl Site {
  pub fn new() -> Self {
    Self {
      dir: tempfile::tempdir().expect("temp dir"),
      spa: None,
      not_found: None,
      ext: false,
    }
  }

  pub fn path(&self) -> &Path {
    self.dir.path()
  }

  /// Write a file, creating whatever directories it needs on the way.
  pub fn file(self, rel: &str, contents: impl AsRef<[u8]>) -> Self {
    let path = self.dir.path().join(rel);
    if let Some(parent) = path.parent() {
      std::fs::create_dir_all(parent).expect("create parent");
    }
    std::fs::write(&path, contents).expect("write file");
    self
  }

  pub fn folder(self, rel: &str) -> Self {
    std::fs::create_dir_all(self.dir.path().join(rel)).expect("create dir");
    self
  }

  /// `--spa`, resolved against the root the way main.rs resolves it.
  pub fn spa(mut self, file: &str) -> Self {
    self.spa = Some(PathBuf::from(file));
    self
  }

  /// `--not-found`.
  pub fn not_found(mut self, file: &str) -> Self {
    self.not_found = Some(PathBuf::from(file));
    self
  }

  /// `-e`: serve paths exactly as written.
  pub fn ext(mut self) -> Self {
    self.ext = true;
    self
  }

  pub fn config(&self) -> Arc<Config> {
    // main.rs canonicalises the root before anything else, and `is_within`
    // compares against a canonical path, so the tests must too — a macOS temp
    // directory lives under a symlinked `/var`.
    let root = self.dir.path().canonicalize().expect("canonical root");
    Arc::new(Config {
      spa: self.spa.as_ref().map(|p| root.join(p)),
      not_found: self.not_found.as_ref().map(|p| root.join(p)),
      ext: self.ext,
      root,
    })
  }

  /// Send one request and read the whole reply back.
  pub async fn send(&self, req: Req) -> Reply {
    send(&self.config(), req).await
  }

  pub async fn get(&self, path: &str) -> Reply {
    self.send(Req::get(path)).await
  }
}

/// A request, assembled as the bytes that go down the socket.
pub struct Req {
  method: String,
  path: String,
  headers: Vec<(String, String)>,
}

impl Req {
  pub fn get(path: &str) -> Self {
    Self::new("GET", path)
  }

  pub fn head(path: &str) -> Self {
    Self::new("HEAD", path)
  }

  pub fn new(method: &str, path: &str) -> Self {
    Self {
      method: method.to_string(),
      path: path.to_string(),
      headers: Vec::new(),
    }
  }

  pub fn header(mut self, name: &str, value: &str) -> Self {
    self.headers.push((name.to_string(), value.to_string()));
    self
  }

  fn wire(&self) -> String {
    let mut out = format!("{} {} HTTP/1.1\r\n", self.method, self.path);
    if !self
      .headers
      .iter()
      .any(|(n, _)| n.eq_ignore_ascii_case("host"))
    {
      // A fixed default keeps the built-in 404 page — which echoes the Host —
      // the same from one run to the next.
      out.push_str("Host: 127.0.0.1:8010\r\n");
    }
    for (name, value) in &self.headers {
      out.push_str(&format!("{name}: {value}\r\n"));
    }
    // One request per connection, so the server closes and the client can read
    // to EOF without parsing a length.
    out.push_str("Connection: close\r\n\r\n");
    out
  }
}

/// What came back, kept as bytes because a gzipped body is not text.
pub struct Reply {
  pub status: u16,
  pub headers: Vec<(String, String)>,
  pub body: Vec<u8>,
}

impl Reply {
  pub fn header(&self, name: &str) -> Option<&str> {
    self
      .headers
      .iter()
      .find(|(n, _)| n.eq_ignore_ascii_case(name))
      .map(|(_, v)| v.as_str())
  }

  pub fn has(&self, name: &str) -> bool {
    self.header(name).is_some()
  }

  pub fn text(&self) -> String {
    String::from_utf8_lossy(&self.body).into_owned()
  }

  /// The declared length, which for a HEAD is not the length of `body`.
  pub fn content_length(&self) -> Option<u64> {
    self.header("content-length").and_then(|v| v.parse().ok())
  }

  pub fn gunzip(&self) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(&self.body[..])
      .read_to_end(&mut out)
      .expect("a gzip body");
    out
  }
}

pub async fn send(config: &Arc<Config>, req: Req) -> Reply {
  let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
  let addr = listener.local_addr().expect("local addr");

  let config = Arc::clone(config);
  let server = tokio::spawn(async move {
    let (stream, _) = listener.accept().await.expect("accept");
    let service = service_fn(move |r| serve::handle(r, Arc::clone(&config)));
    // A client that closes first is not an error worth failing a test over.
    let _ = http1::Builder::new()
      .serve_connection(TokioIo::new(stream), service)
      .await;
  });

  let mut stream = TcpStream::connect(addr).await.expect("connect");
  stream
    .write_all(req.wire().as_bytes())
    .await
    .expect("write request");

  let mut raw = Vec::new();
  stream.read_to_end(&mut raw).await.expect("read response");
  server.await.expect("server task");

  parse(&raw)
}

fn parse(raw: &[u8]) -> Reply {
  let split = raw
    .windows(4)
    .position(|w| w == b"\r\n\r\n")
    .expect("a complete header block");
  let head = std::str::from_utf8(&raw[..split]).expect("headers are text");
  let body = raw[split + 4..].to_vec();

  let mut lines = head.split("\r\n");
  let status = lines
    .next()
    .and_then(|line| line.split_whitespace().nth(1))
    .and_then(|code| code.parse().ok())
    .expect("a status line");

  let headers = lines
    .filter_map(|line| line.split_once(':'))
    .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
    .collect();

  Reply {
    status,
    headers,
    body,
  }
}
