use base64::prelude::*;
use clap::Parser;
use futures::try_join;
use serde::{Deserialize, Serialize};
use std::fs;
use std::ops::Deref;
use std::str;

use axum::{
    extract::{FromRef, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use axum_extra::extract::Query;
use std::net::SocketAddr;

use maud::html;

mod html;
mod rotki;
mod simplefin_api;
mod svg_icon;
use rust_embed::RustEmbed;

#[derive(RustEmbed, Clone)]
#[folder = "static/"]
struct StaticAssets;

#[derive(Parser, Debug)]
pub struct Args {
    #[arg(short, long, default_value = "127.0.0.1:3001")]
    bind_addr: String,
    #[arg(short, long, default_value = "simplefin-rotki.toml")]
    config_file: String,
    #[arg(short, long, value_enum, default_value = "DEBUG")]
    log_level: tracing::Level,
    #[arg(long, action)]
    log_json: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct AppConfig {
    //auth: auth::AuthConfig,
    url: String,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    service_conventions::tracing::setup(args.log_level);

    let config_file_error_msg = format!("Could not read config file {}", args.config_file);
    let config_file_contents = fs::read_to_string(args.config_file).expect(&config_file_error_msg);

    let app_config: AppConfig =
        toml::from_str(&config_file_contents).expect("Problems parsing config file");

    // Start by making a database connection.

    let serve_assets = axum_embed::ServeEmbed::<StaticAssets>::new();
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/", get(get_root))
        .route("/f/:b64_rotki_url", post(post_exchange_token))
        .route("/f/:b64_rotki_url/accounts", get(get_rotki))
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(app_config.clone())
        .layer(service_conventions::tracing_http::trace_layer(
            tracing::Level::INFO,
        ))
        .route("/_health", get(health));

    let addr: SocketAddr = args.bind_addr.parse().expect("Expected bind addr");
    tracing::info!("listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn health() -> Response {
    "OK".into_response()
}

#[derive(Clone, Debug, Deserialize)]
struct RotkiUrl {
    rotki_url: String,
}

async fn get_root(config: State<AppConfig>, query_args: Option<Query<RotkiUrl>>) -> Response {
    let mut b64_sync_url: Option<String> = None;
    if let Some(r_url) = query_args {
        let rotki_url = r_url.rotki_url.clone();
        let b64_rotki_url = BASE64_STANDARD.encode(rotki_url.as_bytes());
        let app_url = config.url.clone();
        let sync_url = format!("{app_url}/f/{b64_rotki_url}");
        b64_sync_url = Some(BASE64_STANDARD.encode(sync_url.as_bytes()));
        tracing::debug!(url = sync_url, token = b64_sync_url, "Token")
    }
    html::maud_page(maud::html!(
    form action="/" {
        input name="rotki_url" {}
    }

    @if let Some(token_url) = b64_sync_url {
        p { "Token URL: " (token_url)}
    }

    ))
    .into_response()
}

#[derive(Clone, Debug, Deserialize)]
struct B64RotkiURL {
    b64_rotki_url: String,
}

async fn get_rotki(Path(params): Path<B64RotkiURL>) -> Result<Response, AppError> {
    let r_url_bytes = BASE64_STANDARD.decode(&params.b64_rotki_url)?;
    let r_url = String::from_utf8_lossy(&r_url_bytes).into_owned();
    let r = rotki::RotkiAPI::new(r_url);
    let balances = r.balances().await?;
    let sf = balances_to_sf(balances)?;

    Ok(axum::Json(sf).into_response())
}

// Return the URL for this handler as the SimpleFin protocol POST's to here, but we don't need to
// do anything with it. Future GETs to the URL will fetch and process the Rotki data
async fn post_exchange_token(
    config: State<AppConfig>,
    Path(params): Path<B64RotkiURL>,
) -> Result<Response, AppError> {
    // Decode to ensure it's somewhat valid at this stage
    BASE64_STANDARD.decode(&params.b64_rotki_url)?;

    let app_url = config.url.clone();
    let sync_url = format!("{app_url}/f/{}", &params.b64_rotki_url);

    Ok(sync_url.into_response())
}

fn balances_to_sf(b: rotki::RotkiBalanceResponse) -> anyhow::Result<simplefin_api::AccountSet> {
    let now = chrono::Utc::now();
    let org = simplefin_api::Organization {
        domain: None,
        name: None,
        sfin_url: "https://example.com/sfn_url".to_string(),
    };
    let accounts = b.result.location.into_iter();
    let sfa = accounts
        .map(|(name, data)| simplefin_api::Account {
            org: org.clone(),
            balance: data.usd_value,
            available_balance: None,
            balance_date: now.clone(),

            id: name.clone(),
            name,
            currency: simplefin_api::Currency::from("usd"),
            transactions: vec![],
        })
        .collect();
    Ok(simplefin_api::AccountSet {
        errors: vec![],
        accounts: sfa,
    })
}

// Make our own error that wraps `anyhow::Error`.
#[derive(Debug)]
struct AppError(anyhow::Error);

// Tell axum how to convert `AppError` into a response.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!(e = ?self, "Error");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", self.0),
        )
            .into_response()
    }
}

// This enables using `?` on functions that return `Result<_, anyhow::Error>` to turn them into
// `Result<_, AppError>`. That way you don't need to do that manually.
impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}
