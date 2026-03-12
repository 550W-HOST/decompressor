use std::fmt;
use std::io;

use async_compression::tokio::bufread::GzipDecoder;
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::header::{
  CONTENT_ENCODING, CONTENT_LENGTH, HOST, HeaderMap, HeaderName, HeaderValue,
};
use axum::http::{Request, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use futures_util::TryStreamExt;
use reqwest::Url;
use tokio::io::BufReader;
use tokio_util::io::{ReaderStream, StreamReader};

#[derive(Clone)]
pub struct AppState {
  client: reqwest::Client,
  upstream_base_url: Url,
}

impl AppState {
  pub fn new(upstream_base_url: Url) -> Result<Self, reqwest::Error> {
    let client = reqwest::Client::builder()
      .redirect(reqwest::redirect::Policy::none())
      .build()?;

    Ok(Self {
      client,
      upstream_base_url,
    })
  }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum RequestContentEncoding {
  Identity,
  Gzip,
}

pub fn app(state: AppState) -> Router {
  Router::new().fallback(any(proxy_request)).with_state(state)
}

async fn proxy_request(
  State(state): State<AppState>,
  request: Request<Body>,
) -> Result<Response, ProxyError> {
  let (parts, body) = request.into_parts();
  let upstream_url = build_upstream_url(&state.upstream_base_url, &parts.uri)?;
  let method =
    reqwest::Method::from_bytes(parts.method.as_str().as_bytes()).map_err(ProxyError::Method)?;
  let content_encoding = parse_request_content_encoding(&parts.headers)?;
  let upstream_body = request_body_for_upstream(body, content_encoding);
  let upstream_headers = sanitize_request_headers(&parts.headers, content_encoding)?;

  let upstream_response = state
    .client
    .request(method, upstream_url)
    .headers(upstream_headers)
    .body(upstream_body)
    .send()
    .await
    .map_err(ProxyError::Upstream)?;

  Ok(response_from_upstream(upstream_response))
}

fn build_upstream_url(upstream_base_url: &Url, request_uri: &Uri) -> Result<Url, ProxyError> {
  let path_and_query = request_uri
    .path_and_query()
    .map(|value| value.as_str())
    .unwrap_or("/");

  upstream_base_url
    .join(path_and_query)
    .map_err(|error| ProxyError::InvalidUpstreamUrl(error.to_string()))
}

fn parse_request_content_encoding(
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
  } else {
    Err(ProxyError::UnsupportedContentEncoding(values.join(", ")))
  }
}

fn sanitize_request_headers(
  original_headers: &HeaderMap,
  _content_encoding: RequestContentEncoding,
) -> Result<HeaderMap, ProxyError> {
  let mut headers = HeaderMap::new();

  for (name, value) in original_headers {
    if is_hop_by_hop_header(name)
      || name == HOST
      || name == CONTENT_LENGTH
      || name == CONTENT_ENCODING
    {
      continue;
    }

    headers.append(name, clone_header_value(value)?);
  }

  Ok(headers)
}

fn request_body_for_upstream(
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
  }
}

fn response_from_upstream(upstream_response: reqwest::Response) -> Response {
  let status = upstream_response.status();
  let upstream_headers = upstream_response.headers().clone();
  let mut response = Response::new(Body::from_stream(upstream_response.bytes_stream()));
  *response.status_mut() = status;

  let response_headers = response.headers_mut();

  for (name, value) in &upstream_headers {
    if !is_hop_by_hop_header(name) {
      response_headers.append(name, value.clone());
    }
  }

  response
}

fn clone_header_value(value: &HeaderValue) -> Result<HeaderValue, ProxyError> {
  HeaderValue::from_bytes(value.as_bytes()).map_err(ProxyError::HeaderValue)
}

fn is_hop_by_hop_header(header_name: &HeaderName) -> bool {
  matches!(
    header_name.as_str(),
    "connection"
      | "keep-alive"
      | "proxy-authenticate"
      | "proxy-authorization"
      | "te"
      | "trailer"
      | "transfer-encoding"
      | "upgrade"
  )
}

#[derive(Debug)]
pub enum ProxyError {
  HeaderValue(axum::http::header::InvalidHeaderValue),
  InvalidUpstreamUrl(String),
  Method(axum::http::method::InvalidMethod),
  UnsupportedContentEncoding(String),
  Upstream(reqwest::Error),
}

impl IntoResponse for ProxyError {
  fn into_response(self) -> Response {
    let status = match self {
      Self::UnsupportedContentEncoding(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
      Self::Upstream(_) => StatusCode::BAD_GATEWAY,
      Self::HeaderValue(_) | Self::InvalidUpstreamUrl(_) | Self::Method(_) => {
        StatusCode::INTERNAL_SERVER_ERROR
      }
    };

    (status, self.to_string()).into_response()
  }
}

impl fmt::Display for ProxyError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::HeaderValue(error) => write!(f, "invalid header value: {error}"),
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
