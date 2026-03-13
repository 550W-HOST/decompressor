use std::error::Error as StdError;
use std::fmt;
use std::io;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug)]
pub enum ProxyError {
  HeaderValue(axum::http::header::InvalidHeaderValue),
  InvalidForwardedHeaderValue(String),
  InvalidUpstreamUrl(String),
  Method(axum::http::method::InvalidMethod),
  UnsupportedContentEncoding(String),
  Upstream(reqwest::Error),
}

impl IntoResponse for ProxyError {
  fn into_response(self) -> Response {
    let status = match &self {
      Self::UnsupportedContentEncoding(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
      Self::Upstream(error) if error.is_body() => StatusCode::BAD_REQUEST,
      Self::Upstream(error) if is_invalid_request_stream_error(error) => StatusCode::BAD_REQUEST,
      Self::Upstream(error) if error.is_timeout() => StatusCode::GATEWAY_TIMEOUT,
      Self::Upstream(_) => StatusCode::BAD_GATEWAY,
      Self::HeaderValue(_)
      | Self::InvalidForwardedHeaderValue(_)
      | Self::InvalidUpstreamUrl(_)
      | Self::Method(_) => StatusCode::INTERNAL_SERVER_ERROR,
    };

    (status, self.to_string()).into_response()
  }
}

impl fmt::Display for ProxyError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::HeaderValue(error) => write!(f, "invalid header value: {error}"),
      Self::InvalidForwardedHeaderValue(name) => {
        write!(f, "invalid forwarded header value: {name}")
      }
      Self::InvalidUpstreamUrl(error) => write!(f, "failed to build upstream URL: {error}"),
      Self::Method(error) => write!(f, "invalid request method: {error}"),
      Self::UnsupportedContentEncoding(value) => {
        write!(f, "unsupported content-encoding: {value}")
      }
      Self::Upstream(error) => write!(f, "upstream request failed: {error}"),
    }
  }
}

impl std::error::Error for ProxyError {}

fn is_invalid_request_stream_error(error: &reqwest::Error) -> bool {
  let mut source: Option<&(dyn StdError + 'static)> = error.source();

  while let Some(current) = source {
    if let Some(io_error) = current.downcast_ref::<io::Error>() {
      if matches!(
        io_error.kind(),
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
      ) {
        return true;
      }
    }

    source = current.source();
  }

  false
}
