use axum::body::{self, Body};
use axum::http::header::{CONTENT_ENCODING, CONTENT_TYPE, HeaderValue};
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::{Json, Router};
use decompressor::proxy::{AppState, app};
use libdeflater::{CompressionLvl, Compressor};
use serde_json::{Value, json};

#[tokio::test]
async fn forwards_plain_json_requests_unchanged() {
  let upstream = spawn_server(upstream_app()).await;
  let proxy = spawn_server(app(
    AppState::new(upstream.base_url.parse().expect("invalid upstream URL")).unwrap(),
  ))
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
  assert_eq!(body["body"], request_body);

  proxy.abort();
  upstream.abort();
}

#[tokio::test]
async fn decompresses_gzip_request_bodies_before_forwarding() {
  let upstream = spawn_server(upstream_app()).await;
  let proxy = spawn_server(app(
    AppState::new(upstream.base_url.parse().expect("invalid upstream URL")).unwrap(),
  ))
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

fn upstream_app() -> Router {
  Router::new().fallback(any(echo_request))
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

  let response_body = json!({
    "method": parts.method.as_str(),
    "path": parts.uri.path(),
    "query": parts.uri.query(),
    "content_encoding": content_encoding,
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
    axum::serve(listener, router)
      .await
      .expect("test server exited unexpectedly");
  });

  TestServer {
    base_url: format!("http://{address}"),
    handle,
  }
}
