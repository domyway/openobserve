// Copyright 2025 OpenObserve Inc.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

use std::{
    error::Error as StdError,
    future::Future,
    io::{self, Write as _},
    pin::Pin,
    task::{Context, Poll},
};

use actix_http::{
    ResponseHead, StatusCode,
    body::{self, BodySize, MessageBody},
    header::{self, CONTENT_ENCODING, ContentEncoding, HeaderValue},
};
use actix_web::rt::task::{JoinHandle, spawn_blocking};
use bytes::Bytes;
use derive_more::Display;
use flate2::write::{GzEncoder, ZlibEncoder};
use futures_core::ready;
use pin_project_lite::pin_project;
use tracing::trace;
use zstd::stream::write::Encoder as ZstdEncoder;

use super::Writer;

const MAX_CHUNK_SIZE_ENCODE_IN_PLACE: usize = 1024;

pin_project! {
    pub struct Encoder<B> {
        #[pin]
        body: EncoderBody<B>,
        encoder: Option<ContentEncoder>,
        fut: Option<JoinHandle<Result<ContentEncoder, io::Error>>>,
        eof: bool,
    }
}

impl<B: MessageBody> Encoder<B> {
    fn none() -> Self {
        Encoder {
            body: EncoderBody::None {
                body: body::None::new(),
            },
            encoder: None,
            fut: None,
            eof: true,
        }
    }

    fn empty() -> Self {
        Encoder {
            body: EncoderBody::Full { body: Bytes::new() },
            encoder: None,
            fut: None,
            eof: true,
        }
    }

    pub fn response(encoding: ContentEncoding, head: &mut ResponseHead, body: B) -> Self {
        // no need to compress empty bodies
        match body.size() {
            BodySize::None => return Self::none(),
            BodySize::Sized(0) => return Self::empty(),
            _ => {}
        }

        let should_encode = !(head.headers().contains_key(&CONTENT_ENCODING)
            || head.status == StatusCode::SWITCHING_PROTOCOLS
            || head.status == StatusCode::NO_CONTENT
            || encoding == ContentEncoding::Identity);

        let body = match body.try_into_bytes() {
            Ok(body) => EncoderBody::Full { body },
            Err(body) => EncoderBody::Stream { body },
        };

        if should_encode {
            // wrap body only if encoder is feature-enabled
            if let Some(enc) = ContentEncoder::select(encoding) {
                update_head(encoding, head);

                return Encoder {
                    body,
                    encoder: Some(enc),
                    fut: None,
                    eof: false,
                };
            }
        }

        Encoder {
            body,
            encoder: None,
            fut: None,
            eof: false,
        }
    }
}

pin_project! {
    #[project = EncoderBodyProj]
    enum EncoderBody<B> {
        None { body: body::None },
        Full { body: Bytes },
        Stream { #[pin] body: B },
    }
}

impl<B> MessageBody for EncoderBody<B>
where
    B: MessageBody,
{
    type Error = EncoderError;

    #[inline]
    fn size(&self) -> BodySize {
        match self {
            EncoderBody::None { body } => body.size(),
            EncoderBody::Full { body } => body.size(),
            EncoderBody::Stream { body } => body.size(),
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        match self.project() {
            EncoderBodyProj::None { body } => {
                Pin::new(body).poll_next(cx).map_err(|err| match err {})
            }
            EncoderBodyProj::Full { body } => {
                Pin::new(body).poll_next(cx).map_err(|err| match err {})
            }
            EncoderBodyProj::Stream { body } => body
                .poll_next(cx)
                .map_err(|err| EncoderError::Body(err.into())),
        }
    }

    #[inline]
    fn try_into_bytes(self) -> Result<Bytes, Self>
    where
        Self: Sized,
    {
        match self {
            EncoderBody::None { body } => Ok(body.try_into_bytes().unwrap()),
            EncoderBody::Full { body } => Ok(body.try_into_bytes().unwrap()),
            _ => Err(self),
        }
    }
}

impl<B> MessageBody for Encoder<B>
where
    B: MessageBody,
{
    type Error = EncoderError;

    #[inline]
    fn size(&self) -> BodySize {
        if self.encoder.is_some() {
            BodySize::Stream
        } else {
            self.body.size()
        }
    }

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Bytes, Self::Error>>> {
        let mut this = self.project();

        loop {
            if *this.eof {
                return Poll::Ready(None);
            }

            if let Some(fut) = this.fut {
                let mut encoder = ready!(Pin::new(fut).poll(cx))
                    .map_err(|_| {
                        EncoderError::Io(io::Error::other(
                            "Blocking task was cancelled unexpectedly",
                        ))
                    })?
                    .map_err(EncoderError::Io)?;

                let chunk = encoder.take();
                *this.encoder = Some(encoder);
                this.fut.take();

                if !chunk.is_empty() {
                    return Poll::Ready(Some(Ok(chunk)));
                }
            }

            let result = ready!(this.body.as_mut().poll_next(cx));

            match result {
                Some(Err(err)) => return Poll::Ready(Some(Err(err))),

                Some(Ok(chunk)) => {
                    // log::debug!(
                    //     "chunk: {:?}, {}",
                    //     chunk.len(),
                    //     String::from_utf8_lossy(&chunk)
                    // );
                    if let Some(mut encoder) = this.encoder.take() {
                        if chunk.len() < MAX_CHUNK_SIZE_ENCODE_IN_PLACE {
                            encoder.write(&chunk).map_err(EncoderError::Io)?;
                            let chunk = encoder.flush().map_err(EncoderError::Io)?;
                            let _ = encoder.take();
                            *this.encoder = Some(encoder);

                            if !chunk.is_empty() {
                                return Poll::Ready(Some(Ok(chunk)));
                            }
                        } else {
                            *this.fut = Some(spawn_blocking(move || {
                                encoder.write(&chunk)?;
                                Ok(encoder)
                            }));
                        }
                    } else {
                        return Poll::Ready(Some(Ok(chunk)));
                    }
                }

                None => {
                    if let Some(encoder) = this.encoder.take() {
                        let chunk = encoder.finish().map_err(EncoderError::Io)?;

                        if chunk.is_empty() {
                            return Poll::Ready(None);
                        } else {
                            *this.eof = true;
                            return Poll::Ready(Some(Ok(chunk)));
                        }
                    } else {
                        return Poll::Ready(None);
                    }
                }
            }
        }
    }

    #[inline]
    fn try_into_bytes(mut self) -> Result<Bytes, Self>
    where
        Self: Sized,
    {
        if self.encoder.is_some() {
            Err(self)
        } else {
            match self.body.try_into_bytes() {
                Ok(body) => Ok(body),
                Err(body) => {
                    self.body = body;
                    Err(self)
                }
            }
        }
    }
}

fn update_head(encoding: ContentEncoding, head: &mut ResponseHead) {
    head.headers_mut()
        .insert(header::CONTENT_ENCODING, encoding.to_header_value());
    head.headers_mut()
        .append(header::VARY, HeaderValue::from_static("accept-encoding"));

    head.no_chunking(false);
}

enum ContentEncoder {
    Deflate(ZlibEncoder<Writer>),
    Gzip(GzEncoder<Writer>),
    Brotli(Box<brotli::CompressorWriter<Writer>>),
    // We need explicit 'static lifetime here because ZstdEncoder needs a lifetime argument and
    // we use `spawn_blocking` in `Encoder::poll_next` that requires `FnOnce() -> R + Send +
    // 'static`.
    Zstd(ZstdEncoder<'static, Writer>),
}

impl ContentEncoder {
    fn select(encoding: ContentEncoding) -> Option<Self> {
        match encoding {
            ContentEncoding::Deflate => Some(ContentEncoder::Deflate(ZlibEncoder::new(
                Writer::new(),
                flate2::Compression::fast(),
            ))),
            ContentEncoding::Gzip => Some(ContentEncoder::Gzip(GzEncoder::new(
                Writer::new(),
                flate2::Compression::fast(),
            ))),
            ContentEncoding::Brotli => Some(ContentEncoder::Brotli(new_brotli_compressor())),
            ContentEncoding::Zstd => {
                let encoder = ZstdEncoder::new(Writer::new(), 3).ok()?;
                Some(ContentEncoder::Zstd(encoder))
            }

            _ => None,
        }
    }

    #[inline]
    pub(crate) fn take(&mut self) -> Bytes {
        match *self {
            ContentEncoder::Brotli(ref mut encoder) => encoder.get_mut().take(),
            ContentEncoder::Deflate(ref mut encoder) => encoder.get_mut().take(),
            ContentEncoder::Gzip(ref mut encoder) => encoder.get_mut().take(),
            ContentEncoder::Zstd(ref mut encoder) => encoder.get_mut().take(),
        }
    }

    fn flush(&mut self) -> Result<Bytes, io::Error> {
        match self {
            ContentEncoder::Brotli(encoder) => match encoder.flush() {
                Ok(()) => Ok(encoder.get_mut().take()),
                Err(err) => Err(err),
            },
            ContentEncoder::Gzip(encoder) => match encoder.flush() {
                Ok(()) => Ok(encoder.get_mut().take()),
                Err(err) => Err(err),
            },
            ContentEncoder::Deflate(encoder) => match encoder.flush() {
                Ok(()) => Ok(encoder.get_mut().take()),
                Err(err) => Err(err),
            },
            ContentEncoder::Zstd(encoder) => match encoder.flush() {
                Ok(()) => Ok(encoder.get_mut().take()),
                Err(err) => Err(err),
            },
        }
    }

    fn finish(self) -> Result<Bytes, io::Error> {
        match self {
            ContentEncoder::Brotli(mut encoder) => match encoder.flush() {
                Ok(()) => Ok(encoder.into_inner().buf.freeze()),
                Err(err) => Err(err),
            },
            ContentEncoder::Gzip(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },
            ContentEncoder::Deflate(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },
            ContentEncoder::Zstd(encoder) => match encoder.finish() {
                Ok(writer) => Ok(writer.buf.freeze()),
                Err(err) => Err(err),
            },
        }
    }

    fn write(&mut self, data: &[u8]) -> Result<(), io::Error> {
        match *self {
            ContentEncoder::Brotli(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding br encoding: {}", err);
                    Err(err)
                }
            },
            ContentEncoder::Gzip(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding gzip encoding: {}", err);
                    Err(err)
                }
            },
            ContentEncoder::Deflate(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding deflate encoding: {}", err);
                    Err(err)
                }
            },
            ContentEncoder::Zstd(ref mut encoder) => match encoder.write_all(data) {
                Ok(_) => Ok(()),
                Err(err) => {
                    trace!("Error decoding ztsd encoding: {}", err);
                    Err(err)
                }
            },
        }
    }
}

fn new_brotli_compressor() -> Box<brotli::CompressorWriter<Writer>> {
    Box::new(brotli::CompressorWriter::new(
        Writer::new(),
        32 * 1024, // 32 KiB buffer
        3,         // BROTLI_PARAM_QUALITY
        22,        // BROTLI_PARAM_LGWIN
    ))
}

#[derive(Debug, Display)]
#[non_exhaustive]
pub enum EncoderError {
    /// Wrapped body stream error.
    #[display("body")]
    Body(Box<dyn StdError>),

    /// Generic I/O error.
    #[display("io")]
    Io(io::Error),
}

impl StdError for EncoderError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            EncoderError::Body(err) => Some(&**err),
            EncoderError::Io(err) => Some(err),
        }
    }
}

impl From<EncoderError> for actix_web::Error {
    fn from(err: EncoderError) -> Self {
        actix_web::error::ErrorInternalServerError(err)
    }
}
