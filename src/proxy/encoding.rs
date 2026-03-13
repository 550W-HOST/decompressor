use std::fmt;
use std::io;

use async_compression::tokio::bufread::{BrotliDecoder, GzipDecoder};
use axum::body::{Body, BodyDataStream, Bytes};
use axum::http::header::{CONTENT_ENCODING, HeaderMap};
use futures_util::TryStreamExt;
use tokio::io::{AsyncRead, BufReader};
use tokio_util::io::{ReaderStream, StreamReader};

use super::ProxyError;

type EncodedBodyStream = futures_util::stream::MapErr<BodyDataStream, fn(axum::Error) -> io::Error>;
type EncodedBodyReader = BufReader<StreamReader<EncodedBodyStream, Bytes>>;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(super) enum RequestContentEncoding {
  Identity,
  Gzip,
  Brotli,
}

impl fmt::Display for RequestContentEncoding {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let value = match self {
      Self::Identity => "identity",
      Self::Gzip => "gzip",
      Self::Brotli => "br",
    };

    f.write_str(value)
  }
}

pub(super) fn parse_request_content_encoding(
  headers: &HeaderMap,
) -> Result<RequestContentEncoding, ProxyError> {
  let mut values = Vec::new();

  for value in &headers.get_all(CONTENT_ENCODING) {
    let value = value
      .to_str()
      .map_err(|_| ProxyError::UnsupportedContentEncoding("<non-utf8>".to_owned()))?;

    for token in value.split(',') {
      let token = token.trim().to_ascii_lowercase();
      if !token.is_empty() {
        values.push(token);
      }
    }
  }

  if values.is_empty() || (values.len() == 1 && values[0] == "identity") {
    Ok(RequestContentEncoding::Identity)
  } else if values.len() == 1 && values[0] == "gzip" {
    Ok(RequestContentEncoding::Gzip)
  } else if values.len() == 1 && values[0] == "br" {
    Ok(RequestContentEncoding::Brotli)
  } else {
    Err(ProxyError::UnsupportedContentEncoding(values.join(", ")))
  }
}

pub(super) fn request_body_for_upstream(
  body: Body,
  content_encoding: RequestContentEncoding,
) -> reqwest::Body {
  match content_encoding {
    RequestContentEncoding::Identity => reqwest::Body::wrap_stream(
      body
        .into_data_stream()
        .map_err(identity_data_stream_error as fn(axum::Error) -> io::Error),
    ),
    RequestContentEncoding::Gzip => decoded_request_body(body, GzipDecoder::new),
    RequestContentEncoding::Brotli => decoded_request_body(body, BrotliDecoder::new),
  }
}

fn decoded_request_body<D, F>(body: Body, decoder: F) -> reqwest::Body
where
  D: AsyncRead + Send + 'static,
  F: FnOnce(EncodedBodyReader) -> D,
{
  let input_stream = body
    .into_data_stream()
    .map_err(invalid_data_stream_error as fn(axum::Error) -> io::Error);
  let reader = BufReader::new(StreamReader::new(input_stream));
  let output_stream = ReaderStream::new(decoder(reader));

  reqwest::Body::wrap_stream(output_stream)
}

fn identity_data_stream_error(error: axum::Error) -> io::Error {
  io::Error::other(error.to_string())
}

fn invalid_data_stream_error(error: axum::Error) -> io::Error {
  io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
