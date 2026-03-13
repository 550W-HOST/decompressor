use std::fmt;
use std::io;

use async_compression::tokio::bufread::{BrotliDecoder, GzipDecoder};
use axum::body::Body;
use axum::http::header::{CONTENT_ENCODING, HeaderMap};
use futures_util::TryStreamExt;
use tokio::io::BufReader;
use tokio_util::io::{ReaderStream, StreamReader};

use super::ProxyError;

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
        .map_err(|error| io::Error::other(error.to_string())),
    ),
    RequestContentEncoding::Gzip => {
      let input_stream = body
        .into_data_stream()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()));
      let reader = StreamReader::new(input_stream);
      let decoder = GzipDecoder::new(BufReader::new(reader));
      let output_stream = ReaderStream::new(decoder);

      reqwest::Body::wrap_stream(output_stream)
    }
    RequestContentEncoding::Brotli => {
      let input_stream = body
        .into_data_stream()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()));
      let reader = StreamReader::new(input_stream);
      let decoder = BrotliDecoder::new(BufReader::new(reader));
      let output_stream = ReaderStream::new(decoder);

      reqwest::Body::wrap_stream(output_stream)
    }
  }
}
