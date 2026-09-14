use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body as HttpBody, Bytes, Frame};
use tokio::fs::File;
use tokio::io::Take;
use tokio_util::io::ReaderStream;

/// Every response serv produces is erased into this one body type.
pub type Body = BoxBody<Bytes, io::Error>;

pub fn empty() -> Body {
  full(Bytes::new())
}

pub fn full(bytes: impl Into<Bytes>) -> Body {
  Full::new(bytes.into())
    .map_err(|never| match never {})
    .boxed()
}

/// Stream a (possibly partial) file straight from disk, so serving a large
/// video never means holding it in memory.
pub fn file(file: Take<File>) -> Body {
  FileBody {
    inner: Box::pin(ReaderStream::new(file)),
  }
  .boxed()
}

struct FileBody {
  inner: Pin<Box<ReaderStream<Take<File>>>>,
}

impl HttpBody for FileBody {
  type Data = Bytes;
  type Error = io::Error;

  fn poll_frame(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
    match self.inner.as_mut().poll_next(cx) {
      Poll::Ready(Some(Ok(chunk))) => Poll::Ready(Some(Ok(Frame::data(chunk)))),
      Poll::Ready(Some(Err(err))) => Poll::Ready(Some(Err(err))),
      Poll::Ready(None) => Poll::Ready(None),
      Poll::Pending => Poll::Pending,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use tokio::io::{AsyncReadExt, AsyncSeekExt};

  async fn collect(body: Body) -> Vec<u8> {
    body.collect().await.unwrap().to_bytes().to_vec()
  }

  #[tokio::test]
  async fn an_empty_body_carries_no_bytes() {
    assert!(collect(empty()).await.is_empty());
  }

  #[tokio::test]
  async fn a_full_body_carries_exactly_what_it_was_given() {
    assert_eq!(collect(full("hello")).await, b"hello");
    assert_eq!(collect(full(vec![0u8, 1, 2])).await, [0, 1, 2]);
    assert_eq!(collect(full(Bytes::from_static(b"x"))).await, b"x");
  }

  #[tokio::test]
  async fn a_file_body_streams_the_whole_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "abcdefghij").unwrap();

    let file = File::open(&path).await.unwrap();
    assert_eq!(collect(super::file(file.take(10))).await, b"abcdefghij");
  }

  #[tokio::test]
  async fn a_file_body_stops_at_the_take_limit() {
    // This is how a `206` is cut: seek to the start, then take the span.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "abcdefghij").unwrap();

    let mut handle = File::open(&path).await.unwrap();
    handle.seek(io::SeekFrom::Start(2)).await.unwrap();
    assert_eq!(collect(super::file(handle.take(4))).await, b"cdef");
  }

  #[tokio::test]
  async fn a_file_body_delivered_in_pieces_still_adds_up() {
    // Large files are the reason this type exists: the frames arrive in chunks,
    // and nothing but a buffer is ever held.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.bin");
    let data: Vec<u8> = (0..=255u8).cycle().take(256 * 1024).collect();
    std::fs::write(&path, &data).unwrap();

    let file = File::open(&path).await.unwrap();
    let mut body = super::file(file.take(data.len() as u64));

    let mut frames = 0;
    let mut seen = Vec::new();
    while let Some(frame) = body.frame().await {
      seen.extend_from_slice(frame.unwrap().data_ref().unwrap());
      frames += 1;
    }

    assert_eq!(seen, data);
    assert!(
      frames > 1,
      "a 256 KiB file should arrive in more than one frame"
    );
  }

  #[tokio::test]
  async fn taking_nothing_yields_no_frames() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "abc").unwrap();

    let file = File::open(&path).await.unwrap();
    assert!(collect(super::file(file.take(0))).await.is_empty());
  }
}
