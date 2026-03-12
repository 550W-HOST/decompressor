use std::env;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use base64::Engine as _;
use dotenvy::dotenv;
use libdeflater::{CompressionLvl, Compressor};
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_ENCODING, CONTENT_TYPE};
use serde_json::{Value, json};

fn load_config() -> (String, String, String) {
  dotenv().expect("failed to load .env file");

  let api_base = env::var("API_BASE").expect("missing API_BASE in .env");
  let api_key = env::var("API_KEY").expect("missing API_KEY in .env");
  let model = env::var("MODEL").expect("missing MODEL in .env");

  (api_base, api_key, model)
}

fn build_client() -> Client {
  Client::builder()
    .timeout(Duration::from_secs(30))
    .build()
    .expect("failed to build HTTP client")
}

fn gzip_json_bytes_with_libdeflater(value: &Value) -> Vec<u8> {
  let request_json = serde_json::to_vec(value).expect("failed to serialize request body to JSON");

  // Use libdeflater so this tiny payload is actually DEFLATE-compressed
  // instead of being wrapped as a stored block.
  let mut compressor = Compressor::new(CompressionLvl::best());
  let mut gzipped_body = vec![0; compressor.gzip_compress_bound(request_json.len())];
  let compressed_len = compressor
    .gzip_compress(&request_json, &mut gzipped_body)
    .expect("failed to gzip request body");
  gzipped_body.truncate(compressed_len);
  gzipped_body
}

fn gzip_json_bytes_with_gnu_gzip(value: &Value) -> Vec<u8> {
  let request_json = serde_json::to_vec(value).expect("failed to serialize request body to JSON");

  let mut child = Command::new("gzip")
    .args(["-n", "-9", "-c"])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .spawn()
    .expect("failed to spawn gzip");

  child
    .stdin
    .as_mut()
    .expect("failed to open gzip stdin")
    .write_all(&request_json)
    .expect("failed to write JSON body to gzip stdin");

  let output = child
    .wait_with_output()
    .expect("failed to wait for gzip process");

  assert!(
    output.status.success(),
    "gnu gzip failed with status {}",
    output.status
  );

  output.stdout
}

fn compare_gzip_bodies(
  left_name: &str,
  left: &[u8],
  right_name: &str,
  right: &[u8],
) -> Vec<String> {
  let max_len = left.len().max(right.len());
  let mut differences = Vec::new();

  for index in 0..max_len {
    let left_byte = left.get(index).copied();
    let right_byte = right.get(index).copied();

    if left_byte != right_byte {
      differences.push(format!(
        "byte {index}: {left_name}={:?} {right_name}={:?}",
        left_byte.map(|byte| format!("{byte:02x}")),
        right_byte.map(|byte| format!("{byte:02x}"))
      ));
    }
  }

  differences
}

fn raw_curl_for_gzip_body(body: &[u8]) -> String {
  let encoded = base64::engine::general_purpose::STANDARD.encode(body);
  format!(
    "printf '%s' '{encoded}' | base64 -d | curl 'https://your-api.example.com/v1/chat/completions' -H 'Authorization: Bearer ${TEST_API_KEY:?set TEST_API_KEY}' -H 'Content-Type: application/json' -H 'Content-Encoding: gzip' --data-binary @-"
  )
}

#[test]
#[ignore = "Hits an external API. Run explicitly with cargo test --test cpa_gzip_request_body -- --ignored"]
fn sends_plain_json_request_to_gpt_5_2() {
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

  let response = client
    .post(format!("{api_base}/v1/chat/completions"))
    .header(AUTHORIZATION, format!("Bearer {api_key}"))
    .header(CONTENT_TYPE, "application/json")
    .json(&request_body)
    .send()
    .expect("request to CLI proxy failed");

  assert!(
    response.status().is_success(),
    "unexpected status: {}",
    response.status()
  );

  let body: Value = response.json().expect("response was not valid JSON");
  println!(
    "{}",
    serde_json::to_string_pretty(&body).expect("failed to pretty-print JSON response")
  );

  let returned_model = body["model"]
    .as_str()
    .expect("missing response model field");
  assert!(
    returned_model.starts_with(&model),
    "expected model starting with {model}, got {returned_model}"
  );

  let content = body["choices"][0]["message"]["content"]
    .as_str()
    .expect("missing assistant message content");
  assert!(
    !content.trim().is_empty(),
    "assistant content should not be empty"
  );
}

#[test]
#[ignore = "Hits an external API. Run explicitly with cargo test --test cpa_gzip_request_body -- --ignored --nocapture"]
fn sends_libdeflater_gzip_request_body_to_gpt_5_2() {
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

  let gzipped_body = gzip_json_bytes_with_libdeflater(&request_body);

  let response = client
    .post(format!("{api_base}/v1/chat/completions"))
    .header(AUTHORIZATION, format!("Bearer {api_key}"))
    .header(CONTENT_TYPE, "application/json")
    // Tell the upstream server the HTTP request body itself is gzip-compressed.
    .header(CONTENT_ENCODING, "gzip")
    .body(gzipped_body)
    .send()
    .expect("gzip request to CLI proxy failed");

  let status = response.status();
  let headers = format!("{:#?}", response.headers());
  let response_text = response
    .text()
    .expect("failed to read gzip test response body");

  println!("status: {status}");
  println!("headers: {headers}");

  match serde_json::from_str::<Value>(&response_text) {
    Ok(body) => println!(
      "{}",
      serde_json::to_string_pretty(&body).expect("failed to pretty-print gzip test JSON response")
    ),
    Err(_) => println!("{response_text}"),
  }

  assert!(
    !response_text.trim().is_empty(),
    "gzip test response body should not be empty"
  );
}

#[test]
fn compares_gnu_gzip_and_libdeflater_request_bodies() {
  #[rustfmt::skip]
  let request_body = json!({
    "messages": [
      {
        "content": "hi",
        "role": "user"
      }
    ],
    "model": "gpt-5.2"
  });

  let gnu_gzip_body = gzip_json_bytes_with_gnu_gzip(&request_body);
  let libdeflater_body = gzip_json_bytes_with_libdeflater(&request_body);
  let differences =
    compare_gzip_bodies("gnu gzip", &gnu_gzip_body, "libdeflater", &libdeflater_body);

  println!("gnu gzip bytes: {:02x?}", gnu_gzip_body);
  println!("libdeflater bytes: {:02x?}", libdeflater_body);
  // Only the fixed 10-byte gzip header differs here.
  // The last header byte is the RFC 1952 OS field:
  // - gnu gzip uses 0x03 for Unix
  // - libdeflater uses 0xff for unknown
  // The DEFLATE payload, CRC32, and original input size are otherwise the same.
  // gnu gzip header (hex): 1f 8b 08 00 00 00 00 00 02 03
  // gnu gzip body (base64):
  // H4sIAAAAAAACA6tWyk0tLk5MTy1WsoquVkrOzytJzStRslLKyFTSUSrKz0kFskuLU4uUamN1lHLzU1JzgALpBSW6pnpGSrUA62EJoz8AAAA=
  println!(
    "gnu gzip raw curl:\n{}",
    raw_curl_for_gzip_body(&gnu_gzip_body)
  );
  // libdeflater gzip header (hex): 1f 8b 08 00 00 00 00 00 02 ff
  // libdeflater gzip body (base64):
  // H4sIAAAAAAAC/6tWyk0tLk5MTy1WsoquVkrOzytJzStRslLKyFTSUSrKz0kFskuLU4uUamN1lHLzU1JzgALpBSW6pnpGSrUA62EJoz8AAAA=
  println!(
    "libdeflater raw curl:\n{}",
    raw_curl_for_gzip_body(&libdeflater_body)
  );

  if differences.is_empty() {
    println!("gzip bodies are identical");
  } else {
    println!("gzip bodies differ at {} byte(s):", differences.len());
    for difference in &differences {
      println!("{difference}");
    }
  }

  assert_eq!(differences.len(), 1, "expected exactly one byte difference");
  assert_eq!(
    differences[0],
    "byte 9: gnu gzip=Some(\"03\") libdeflater=Some(\"ff\")"
  );
}
