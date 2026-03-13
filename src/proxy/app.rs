use std::net::SocketAddr;

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::Request;
use axum::response::Response;
use axum::routing::any;

use super::AppState;
use super::ProxyError;
use super::encoding::{
  RequestContentEncoding, parse_request_content_encoding, request_body_for_upstream,
};
use super::headers::sanitize_request_headers;
use super::upstream::{build_upstream_url, response_from_upstream};

pub fn app(state: AppState) -> Router {
  Router::new().fallback(any(proxy_request)).with_state(state)
}

async fn proxy_request(
  State(state): State<AppState>,
  ConnectInfo(client_addr): ConnectInfo<SocketAddr>,
  request: Request<Body>,
) -> Result<Response, ProxyError> {
  let (parts, body) = request.into_parts();
  let upstream_url = build_upstream_url(&state.upstream_base_url, &parts.uri)?;
  let method =
    reqwest::Method::from_bytes(parts.method.as_str().as_bytes()).map_err(ProxyError::Method)?;
  let content_encoding = parse_request_content_encoding(&parts.headers)?;

  if content_encoding != RequestContentEncoding::Identity {
    println!(
      "decompressing {} request body for {} {}",
      content_encoding,
      method,
      parts
        .uri
        .path_and_query()
        .map(|value| value.as_str())
        .unwrap_or("/")
    );
  }

  let upstream_body = request_body_for_upstream(body, content_encoding);
  let upstream_headers = sanitize_request_headers(&parts.headers, content_encoding, client_addr)?;

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
