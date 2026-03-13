use axum::body::{self, Body};
use axum::http::header::{CONTENT_ENCODING, CONTENT_TYPE, HOST, HeaderName, HeaderValue};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use brotli::CompressorReader;
use decompressor::proxy::{AppState, app};
use libdeflater::{CompressionLvl, Compressor};
use serde_json::{Value, json};
use std::io::Read;
use std::net::SocketAddr;
use std::time::Duration;

static X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
static X_FORWARDED_HOST: HeaderName = HeaderName::from_static("x-forwarded-host");
static X_FORWARDED_PROTO: HeaderName = HeaderName::from_static("x-forwarded-proto");

#[tokio::test]
async fn forwards_plain_json_requests_unchanged() {
  let upstream = spawn_server(upstream_app()).await;
  let proxy =
    spawn_proxy(AppState::new(upstream.base_url.parse().expect("invalid upstream URL")).unwrap())
      .await;

  #[rustfmt::skip]
  let request_body = json!({
    "model": "gpt-5.2",
    "messages": [
      {
        "role": "user",
        "content": "hi"
      }
    ]
  });

  let response = reqwest::Client::new()
    .post(format!("{}/v1/chat/completions?foo=bar", proxy.base_url))
    .header(HOST, "public.example.test")
    .header("x-proxy-test", "plain")
    .json(&request_body)
    .send()
    .await
    .expect("plain request to proxy failed");

  assert_eq!(response.status(), StatusCode::CREATED);
  assert_eq!(
    response
      .headers()
      .get("x-upstream")
      .expect("missing x-upstream header"),
    "ok"
  );

  let body: Value = response.json().await.expect("invalid JSON from upstream");

  assert_eq!(body["method"], "POST");
  assert_eq!(body["path"], "/v1/chat/completions");
  assert_eq!(body["query"], "foo=bar");
  assert_eq!(body["content_encoding"], Value::Null);
  assert_eq!(body["x_proxy_test"], "plain");
  assert_eq!(body["host"], "public.example.test");
  assert_eq!(body["x_forwarded_host"], "public.example.test");
  assert_eq!(body["x_forwarded_proto"], "http");
  assert_eq!(body["x_forwarded_for"], "127.0.0.1");
  assert_eq!(body["body"], request_body);

  proxy.abort();
  upstream.abort();
}

#[tokio::test]
async fn decompresses_gzip_request_bodies_before_forwarding() {
  let upstream = spawn_server(upstream_app()).await;
  let proxy =
    spawn_proxy(AppState::new(upstream.base_url.parse().expect("invalid upstream URL")).unwrap())
      .await;

  #[rustfmt::skip]
  let request_body = json!({
    "model": "gpt-5.2",
    "messages": [
      {
        "role": "user",
        "content": "hi"
      }
    ]
  });

  let gzipped_body = gzip_json_with_libdeflater(&request_body);

  let response = reqwest::Client::new()
    .post(format!("{}/v1/chat/completions?foo=bar", proxy.base_url))
    .header("x-proxy-test", "gzip")
    .header(CONTENT_TYPE, "application/json")
    .header(CONTENT_ENCODING, "gzip")
    .body(gzipped_body)
    .send()
    .await
    .expect("gzip request to proxy failed");

  assert_eq!(response.status(), StatusCode::CREATED);
  assert_eq!(
    response
      .headers()
      .get("x-upstream")
      .expect("missing x-upstream header"),
    "ok"
  );

  let body: Value = response.json().await.expect("invalid JSON from upstream");

  assert_eq!(body["method"], "POST");
  assert_eq!(body["path"], "/v1/chat/completions");
  assert_eq!(body["query"], "foo=bar");
  assert_eq!(body["content_encoding"], Value::Null);
  assert_eq!(body["x_proxy_test"], "gzip");
  assert_eq!(body["body"], request_body);

  proxy.abort();
  upstream.abort();
}

#[tokio::test]
async fn decompresses_brotli_request_bodies_before_forwarding() {
  let upstream = spawn_server(upstream_app()).await;
  let proxy =
    spawn_proxy(AppState::new(upstream.base_url.parse().expect("invalid upstream URL")).unwrap())
      .await;

  #[rustfmt::skip]
  let request_body = json!({
    "model": "gpt-5.2",
    "messages": [
      {
        "role": "user",
        "content": "hi"
      }
    ]
  });

  let brotli_body = brotli_json(&request_body);

  let response = reqwest::Client::new()
    .post(format!("{}/v1/chat/completions?foo=bar", proxy.base_url))
    .header("x-proxy-test", "br")
    .header(CONTENT_TYPE, "application/json")
    .header(CONTENT_ENCODING, "br")
    .body(brotli_body)
    .send()
    .await
    .expect("brotli request to proxy failed");

  assert_eq!(response.status(), StatusCode::CREATED);
  assert_eq!(
    response
      .headers()
      .get("x-upstream")
      .expect("missing x-upstream header"),
    "ok"
  );

  let body: Value = response.json().await.expect("invalid JSON from upstream");

  assert_eq!(body["method"], "POST");
  assert_eq!(body["path"], "/v1/chat/completions");
  assert_eq!(body["query"], "foo=bar");
  assert_eq!(body["content_encoding"], Value::Null);
  assert_eq!(body["x_proxy_test"], "br");
  assert_eq!(body["body"], request_body);

  proxy.abort();
  upstream.abort();
}

#[tokio::test]
async fn passes_upstream_response_status_headers_and_body_through() {
  let upstream = spawn_server(upstream_app()).await;
  let proxy =
    spawn_proxy(AppState::new(upstream.base_url.parse().expect("invalid upstream URL")).unwrap())
      .await;

  let response = reqwest::Client::new()
    .get(format!("{}/response-check", proxy.base_url))
    .send()
    .await
    .expect("response passthrough request failed");

  assert_eq!(response.status(), StatusCode::ACCEPTED);
  assert_eq!(
    response
      .headers()
      .get(CONTENT_TYPE)
      .expect("missing content-type header"),
    "text/plain; charset=utf-8"
  );
  assert_eq!(
    response
      .headers()
      .get("x-upstream-response")
      .expect("missing x-upstream-response header"),
    "passthrough-ok"
  );
  assert_eq!(
    response
      .headers()
      .get("cache-control")
      .expect("missing cache-control header"),
    "no-store"
  );

  let body = response.text().await.expect("failed to read response body");
  assert_eq!(body, "upstream raw body\nsecond line");

  proxy.abort();
  upstream.abort();
}

#[tokio::test]
async fn rejects_invalid_gzip_request_bodies() {
  let upstream = spawn_server(upstream_app()).await;
  let proxy =
    spawn_proxy(AppState::new(upstream.base_url.parse().expect("invalid upstream URL")).unwrap())
      .await;

  let response = reqwest::Client::new()
    .post(format!("{}/invalid-gzip", proxy.base_url))
    .header(CONTENT_TYPE, "application/json")
    .header(CONTENT_ENCODING, "gzip")
    .body("this-is-not-gzip")
    .send()
    .await
    .expect("invalid gzip request should still receive a response");

  assert_eq!(response.status(), StatusCode::BAD_REQUEST);

  proxy.abort();
  upstream.abort();
}

#[tokio::test]
async fn rejects_unsupported_content_encoding() {
  let upstream = spawn_server(upstream_app()).await;
  let proxy =
    spawn_proxy(AppState::new(upstream.base_url.parse().expect("invalid upstream URL")).unwrap())
      .await;

  let response = reqwest::Client::new()
    .post(format!("{}/unsupported-encoding", proxy.base_url))
    .header(CONTENT_TYPE, "application/json")
    .header(CONTENT_ENCODING, "deflate")
    .body("{}")
    .send()
    .await
    .expect("unsupported encoding request failed");

  assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

  proxy.abort();
  upstream.abort();
}

#[tokio::test]
async fn forwards_large_gzip_request_bodies() {
  let upstream = spawn_server(upstream_app()).await;
  let proxy =
    spawn_proxy(AppState::new(upstream.base_url.parse().expect("invalid upstream URL")).unwrap())
      .await;

  #[rustfmt::skip]
  let request_body = json!({
    "model": "gpt-5.2",
    "messages": [
      {
        "role": "user",
        "content": "x".repeat(256 * 1024)
      }
    ]
  });

  let gzipped_body = gzip_json_with_libdeflater(&request_body);

  let response = reqwest::Client::new()
    .post(format!("{}/large-gzip", proxy.base_url))
    .header(CONTENT_TYPE, "application/json")
    .header(CONTENT_ENCODING, "gzip")
    .body(gzipped_body)
    .send()
    .await
    .expect("large gzip request failed");

  assert_eq!(response.status(), StatusCode::CREATED);
  let body: Value = response.json().await.expect("invalid JSON response");
  assert_eq!(body["body"], request_body);

  proxy.abort();
  upstream.abort();
}

#[tokio::test]
async fn returns_bad_gateway_when_upstream_is_unreachable() {
  let unused_address = unused_local_address().await;
  let proxy = spawn_proxy(
    AppState::new(
      format!("http://{unused_address}")
        .parse()
        .expect("invalid upstream URL"),
    )
    .unwrap(),
  )
  .await;

  let response = reqwest::Client::new()
    .get(format!("{}/upstream-down", proxy.base_url))
    .send()
    .await
    .expect("proxy request failed");

  assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

  proxy.abort();
}

#[tokio::test]
async fn returns_gateway_timeout_when_upstream_times_out() {
  let upstream = spawn_server(upstream_app()).await;
  let client = reqwest::Client::builder()
    .timeout(Duration::from_millis(50))
    .build()
    .expect("failed to build reqwest client");
  let proxy = spawn_proxy(
    AppState::with_client(
      client,
      upstream.base_url.parse().expect("invalid upstream URL"),
    )
    .unwrap(),
  )
  .await;

  let response = reqwest::Client::new()
    .get(format!("{}/slow-response", proxy.base_url))
    .send()
    .await
    .expect("timeout request failed");

  assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);

  proxy.abort();
  upstream.abort();
}

#[tokio::test]
async fn forwards_head_requests() {
  let upstream = spawn_server(upstream_app()).await;
  let proxy =
    spawn_proxy(AppState::new(upstream.base_url.parse().expect("invalid upstream URL")).unwrap())
      .await;

  let response = reqwest::Client::new()
    .head(format!("{}/method-check", proxy.base_url))
    .send()
    .await
    .expect("head request failed");

  assert_eq!(response.status(), StatusCode::OK);
  assert_eq!(
    response
      .headers()
      .get("x-upstream-method")
      .expect("missing x-upstream-method header"),
    "HEAD"
  );
  assert_eq!(response.text().await.expect("failed to read HEAD body"), "");

  proxy.abort();
  upstream.abort();
}

#[tokio::test]
async fn forwards_options_requests() {
  let upstream = spawn_server(upstream_app()).await;
  let proxy =
    spawn_proxy(AppState::new(upstream.base_url.parse().expect("invalid upstream URL")).unwrap())
      .await;

  let response = reqwest::Client::new()
    .request(
      reqwest::Method::OPTIONS,
      format!("{}/method-check", proxy.base_url),
    )
    .send()
    .await
    .expect("options request failed");

  assert_eq!(response.status(), StatusCode::OK);
  assert_eq!(
    response
      .headers()
      .get("x-upstream-method")
      .expect("missing x-upstream-method header"),
    "OPTIONS"
  );
  assert_eq!(
    response.text().await.expect("failed to read OPTIONS body"),
    "OPTIONS"
  );

  proxy.abort();
  upstream.abort();
}

fn gzip_json_with_libdeflater(value: &Value) -> Vec<u8> {
  let request_json = serde_json::to_vec(value).expect("failed to serialize request body");
  let mut compressor = Compressor::new(CompressionLvl::best());
  let mut gzipped_body = vec![0; compressor.gzip_compress_bound(request_json.len())];
  let compressed_len = compressor
    .gzip_compress(&request_json, &mut gzipped_body)
    .expect("failed to gzip request body");
  gzipped_body.truncate(compressed_len);
  gzipped_body
}

fn brotli_json(value: &Value) -> Vec<u8> {
  let request_json = serde_json::to_vec(value).expect("failed to serialize request body");
  let mut reader = CompressorReader::new(request_json.as_slice(), 4096, 11, 22);
  let mut compressed = Vec::new();
  reader
    .read_to_end(&mut compressed)
    .expect("failed to brotli-compress request body");
  compressed
}

fn upstream_app() -> Router {
  Router::new()
    .route("/response-check", any(passthrough_response))
    .route("/method-check", any(method_check_response))
    .route("/slow-response", any(slow_response))
    .fallback(any(echo_request))
}

async fn echo_request(request: Request<Body>) -> Response {
  let (parts, body) = request.into_parts();
  let body = body::to_bytes(body, usize::MAX)
    .await
    .expect("failed to read upstream request body");
  let body_json: Value = serde_json::from_slice(&body).expect("upstream body should be JSON");

  let content_encoding = parts
    .headers
    .get(CONTENT_ENCODING)
    .and_then(|value| value.to_str().ok())
    .map(str::to_owned);
  let x_proxy_test = parts
    .headers
    .get("x-proxy-test")
    .and_then(|value| value.to_str().ok())
    .map(str::to_owned)
    .unwrap_or_default();
  let host = parts
    .headers
    .get(HOST)
    .and_then(|value| value.to_str().ok())
    .map(str::to_owned);
  let x_forwarded_for = parts
    .headers
    .get(X_FORWARDED_FOR.clone())
    .and_then(|value| value.to_str().ok())
    .map(str::to_owned);
  let x_forwarded_host = parts
    .headers
    .get(X_FORWARDED_HOST.clone())
    .and_then(|value| value.to_str().ok())
    .map(str::to_owned);
  let x_forwarded_proto = parts
    .headers
    .get(X_FORWARDED_PROTO.clone())
    .and_then(|value| value.to_str().ok())
    .map(str::to_owned);

  let response_body = json!({
    "method": parts.method.as_str(),
    "path": parts.uri.path(),
    "query": parts.uri.query(),
    "content_encoding": content_encoding,
    "host": host,
    "x_forwarded_for": x_forwarded_for,
    "x_forwarded_host": x_forwarded_host,
    "x_forwarded_proto": x_forwarded_proto,
    "x_proxy_test": x_proxy_test,
    "body": body_json
  });

  (
    StatusCode::CREATED,
    [("x-upstream", HeaderValue::from_static("ok"))],
    Json(response_body),
  )
    .into_response()
}

async fn passthrough_response() -> Response {
  (
    StatusCode::ACCEPTED,
    [
      (CONTENT_TYPE.as_str(), "text/plain; charset=utf-8"),
      ("x-upstream-response", "passthrough-ok"),
      ("cache-control", "no-store"),
    ],
    "upstream raw body\nsecond line",
  )
    .into_response()
}

async fn method_check_response(request: Request<Body>) -> Response {
  let method = request.method().as_str().to_owned();
  let method_header = method.clone();

  (
    StatusCode::OK,
    [("x-upstream-method", method_header)],
    method,
  )
    .into_response()
}

async fn slow_response() -> Response {
  tokio::time::sleep(Duration::from_millis(200)).await;
  (StatusCode::OK, "slow").into_response()
}

struct TestServer {
  base_url: String,
  handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
  fn abort(self) {
    self.handle.abort();
  }
}

async fn spawn_server(router: Router) -> TestServer {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
    .await
    .expect("failed to bind test listener");
  let address = listener.local_addr().expect("missing local address");
  let handle = tokio::spawn(async move {
    axum::serve(
      listener,
      router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .expect("test server exited unexpectedly");
  });

  TestServer {
    base_url: format!("http://{address}"),
    handle,
  }
}

async fn spawn_proxy(state: AppState) -> TestServer {
  spawn_server(app(state)).await
}

async fn unused_local_address() -> SocketAddr {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
    .await
    .expect("failed to reserve local port");
  let address = listener
    .local_addr()
    .expect("missing reserved local address");
  drop(listener);
  address
}
