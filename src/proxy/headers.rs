use std::net::SocketAddr;

use axum::http::header::{
  CONTENT_ENCODING, CONTENT_LENGTH, HOST, HeaderMap, HeaderName, HeaderValue,
};

use super::ProxyError;
use super::encoding::RequestContentEncoding;

static X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
static X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
static X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");

pub(super) fn sanitize_request_headers(
  original_headers: &HeaderMap,
  content_encoding: RequestContentEncoding,
  client_addr: SocketAddr,
) -> Result<HeaderMap, ProxyError> {
  let mut headers = HeaderMap::new();

  for (name, value) in original_headers {
    if is_hop_by_hop_header(name) || name == CONTENT_LENGTH {
      continue;
    }

    if matches!(
      content_encoding,
      RequestContentEncoding::Gzip | RequestContentEncoding::Brotli
    ) && name == CONTENT_ENCODING
    {
      continue;
    }

    headers.append(name, clone_header_value(value)?);
  }

  apply_forwarding_headers(&mut headers, original_headers, client_addr)?;

  Ok(headers)
}

pub(super) fn is_hop_by_hop_header(header_name: &HeaderName) -> bool {
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

fn apply_forwarding_headers(
  headers: &mut HeaderMap,
  original_headers: &HeaderMap,
  client_addr: SocketAddr,
) -> Result<(), ProxyError> {
  let forwarded_for = append_csv_header_value(
    headers.get(X_FORWARDED_FOR.clone()),
    &client_addr.ip().to_string(),
  )?;
  headers.insert(X_FORWARDED_FOR.clone(), forwarded_for);

  if !headers.contains_key(X_FORWARDED_PROTO.clone()) {
    headers.insert(X_FORWARDED_PROTO.clone(), HeaderValue::from_static("http"));
  }

  if let Some(host) = original_headers.get(HOST) {
    if !headers.contains_key(X_FORWARDED_HOST.clone()) {
      headers.insert(X_FORWARDED_HOST.clone(), clone_header_value(host)?);
    }
  }

  Ok(())
}

fn append_csv_header_value(
  existing: Option<&HeaderValue>,
  value: &str,
) -> Result<HeaderValue, ProxyError> {
  match existing {
    Some(existing) => {
      let existing = existing
        .to_str()
        .map_err(|_| ProxyError::InvalidForwardedHeaderValue("x-forwarded-for".to_owned()))?;
      HeaderValue::from_str(&format!("{existing}, {value}")).map_err(ProxyError::HeaderValue)
    }
    None => HeaderValue::from_str(value).map_err(ProxyError::HeaderValue),
  }
}

fn clone_header_value(value: &HeaderValue) -> Result<HeaderValue, ProxyError> {
  HeaderValue::from_bytes(value.as_bytes()).map_err(ProxyError::HeaderValue)
}
