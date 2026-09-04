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
    Full::new(bytes.into()).map_err(|never| match never {}).boxed()
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
