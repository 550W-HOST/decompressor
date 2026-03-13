use axum::body::Body;
use axum::http::Uri;
use axum::response::Response;
use reqwest::Url;

use super::ProxyError;
use super::headers::is_hop_by_hop_header;

pub(super) fn build_upstream_url(
  upstream_base_url: &Url,
  request_uri: &Uri,
) -> Result<Url, ProxyError> {
  let path_and_query = request_uri
    .path_and_query()
    .map(|value| value.as_str())
    .unwrap_or("/");

  upstream_base_url
    .join(path_and_query)
    .map_err(|error| ProxyError::InvalidUpstreamUrl(error.to_string()))
}

pub(super) fn response_from_upstream(upstream_response: reqwest::Response) -> Response {
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
