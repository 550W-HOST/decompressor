use reqwest::Url;

#[derive(Clone)]
pub struct AppState {
  pub(crate) client: reqwest::Client,
  pub(crate) upstream_base_url: Url,
}

impl AppState {
  pub fn new(upstream_base_url: Url) -> Result<Self, reqwest::Error> {
    let client = reqwest::Client::builder()
      .redirect(reqwest::redirect::Policy::none())
      .build()?;

    Self::with_client(client, upstream_base_url)
  }

  pub fn with_client(
    client: reqwest::Client,
    upstream_base_url: Url,
  ) -> Result<Self, reqwest::Error> {
    Ok(Self {
      client,
      upstream_base_url,
    })
  }
}
