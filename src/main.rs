use decompressor::config::Config;
use decompressor::proxy::{AppState, app};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
  dotenvy::dotenv().ok();

  let config = Config::from_env()?;
  let state = AppState::new(config.upstream_url.clone())?;
  let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;

  println!(
    "decompressor listening on {} -> {}",
    config.listen_addr, config.upstream_url
  );

  axum::serve(listener, app(state)).await?;

  Ok(())
}
