use std::io::Read;
use std::time::Duration;

use brotli::CompressorReader;
use decompressor::config::TestConfig;
use dotenvy::dotenv;
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_ENCODING, CONTENT_TYPE};
use serde_json::{Value, json};

fn load_config() -> (String, String, String) {
  dotenv().expect("failed to load .env file");
  let config = TestConfig::from_env().expect("failed to load test config from .env");

  (config.api_base, config.api_key, config.model)
}

fn build_client() -> Client {
  Client::builder()
    .timeout(Duration::from_secs(30))
    .build()
    .expect("failed to build HTTP client")
}

fn brotli_json_bytes(value: &Value) -> Vec<u8> {
  let request_json = serde_json::to_vec(value).expect("failed to serialize request body to JSON");
  let mut reader = CompressorReader::new(request_json.as_slice(), 4096, 11, 22);
  let mut compressed = Vec::new();

  reader
    .read_to_end(&mut compressed)
    .expect("failed to brotli-compress request body");

  compressed
}

#[test]
#[ignore = "Hits an external API. Run explicitly with cargo test --test cpa_br_request_body -- --ignored --nocapture"]
fn sends_brotli_request_body_to_gpt_5_2() {
  let (api_base, api_key, model) = load_config();
  let client = build_client();

  #[rustfmt::skip]
  let request_body = json!({
    "model": model,
    "messages": [
      {
        "role": "user",
        "content": "hi"
      }
    ]
  });

  let brotli_body = brotli_json_bytes(&request_body);

  let response = client
    .post(format!("{api_base}/v1/chat/completions"))
    .header(AUTHORIZATION, format!("Bearer {api_key}"))
    .header(CONTENT_TYPE, "application/json")
    .header(CONTENT_ENCODING, "br")
    .body(brotli_body)
    .send()
    .expect("brotli request to CLI proxy failed");

  let status = response.status();
  let headers = format!("{:#?}", response.headers());
  let response_text = response
    .text()
    .expect("failed to read brotli test response body");

  println!("status: {status}");
  println!("headers: {headers}");

  match serde_json::from_str::<Value>(&response_text) {
    Ok(body) => println!(
      "{}",
      serde_json::to_string_pretty(&body)
        .expect("failed to pretty-print brotli test JSON response")
    ),
    Err(_) => println!("{response_text}"),
  }

  assert!(
    !response_text.trim().is_empty(),
    "brotli test response body should not be empty"
  );
}
