use std::env;
use std::fmt;
use std::net::SocketAddr;

use reqwest::Url;

const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:5505";
const DEFAULT_UPSTREAM_URL: &str = "http://127.0.0.1:8080";

#[derive(Clone, Debug)]
pub struct Config {
  pub listen_addr: SocketAddr,
  pub upstream_url: Url,
}

impl Config {
  pub fn from_env() -> Result<Self, ConfigError> {
    let listen_addr = env::var("DECOMPRESSOR_LISTEN_ADDR")
      .unwrap_or_else(|_| DEFAULT_LISTEN_ADDR.to_owned())
      .parse()
      .map_err(|_| ConfigError::InvalidListenAddr)?;

    let upstream_url = Url::parse(
      &env::var("DECOMPRESSOR_UPSTREAM_URL").unwrap_or_else(|_| DEFAULT_UPSTREAM_URL.to_owned()),
    )
    .map_err(|error| ConfigError::InvalidUpstreamUrl(error.to_string()))?;

    Ok(Self {
      listen_addr,
      upstream_url,
    })
  }
}

#[derive(Clone, Debug)]
pub struct TestConfig {
  pub api_base: String,
  pub api_key: String,
  pub model: String,
}

impl TestConfig {
  pub fn from_env() -> Result<Self, ConfigError> {
    Ok(Self {
      api_base: env::var("TEST_API_BASE").map_err(|_| ConfigError::MissingVar("TEST_API_BASE"))?,
      api_key: env::var("TEST_API_KEY").map_err(|_| ConfigError::MissingVar("TEST_API_KEY"))?,
      model: env::var("TEST_MODEL").map_err(|_| ConfigError::MissingVar("TEST_MODEL"))?,
    })
  }
}

#[derive(Debug)]
pub enum ConfigError {
  InvalidListenAddr,
  InvalidUpstreamUrl(String),
  MissingVar(&'static str),
}

impl fmt::Display for ConfigError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::InvalidListenAddr => write!(f, "invalid DECOMPRESSOR_LISTEN_ADDR"),
      Self::InvalidUpstreamUrl(error) => write!(f, "invalid DECOMPRESSOR_UPSTREAM_URL: {error}"),
      Self::MissingVar(name) => write!(f, "missing {name} in environment"),
    }
  }
}

impl std::error::Error for ConfigError {}
