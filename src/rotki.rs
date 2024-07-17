use reqwest;
use serde::{Deserialize, Serialize};

pub struct RotkiAPI {
    url: String,
}

impl RotkiAPI {
    pub fn new(url: String) -> Self {
        Self { url }
    }

    pub async fn balances(&self) -> anyhow::Result<RotkiBalanceResponse> {
        let client = reqwest::Client::new();
        let balance_url = format!("{}/api/1/balances", &self.url);
        Ok(request(&client, &balance_url)
            .await?
            .json::<RotkiBalanceResponse>()
            .await?)
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct RotkiBalanceResponse {
    pub result: BalanceResult,
}
#[derive(Deserialize, Serialize, Debug)]
pub struct BalanceResult {
    pub location: std::collections::HashMap<String, BalanceLocationValue>,
}
#[derive(Deserialize, Serialize, Debug)]
pub struct BalanceLocationValue {
    #[serde(with = "rust_decimal::serde::float")]
    pub usd_value: rust_decimal::Decimal,
}

#[tracing::instrument(skip_all)]
async fn request(client: &reqwest::Client, url: &String) -> anyhow::Result<reqwest::Response> {
    tracing::debug!(url = url, "Rotki Request");
    let tracing_headers = service_conventions::tracing_http::get_tracing_headers();
    Ok(client
        .get(url)
        .headers(tracing_headers)
        .header("content-type", "application/json")
        .send()
        .await?)
}
