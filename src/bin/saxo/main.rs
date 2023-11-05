#![feature(async_fn_in_trait, iterator_try_collect)]

use chrono::{DateTime, Duration, Utc};
use log::info;
use pushover::requests::message::SendMessage;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::error::Error;
use std::io::{Read, Write};
use std::{env, net::TcpListener};
use ynab_updater::{update_ynab, GetBalance, GetYnabAccountConfig, YnabAccountConfig};

static SAXO_AUTH_URL: &str = "https://live.logonvalidation.net/authorize";
static SAXO_ACCESS_URL: &str = "https://live.logonvalidation.net/token";
static SAXO_API_URL: &str = "https://gateway.saxobank.com/sim/openapi/";

// TODO: pass in via tailscale ip command?
static TAILSCALE_IP: &str = "100.84.252.9";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Config {
    pub saxo_client_id: String,
    pub saxo_client_secret: String,
    // TODO: pass in via tailscale funnel command?
    pub saxo_redirect_uri: String,
    pub saxo_access_token_path: String,

    pub ynab_saxo_account_id: String,

    pub pushover_user_key: String,
    pub pushover_api_key: String,
}

struct Mock {}

struct Saxo {}

impl GetYnabAccountConfig for Mock {
    async fn get(&self) -> Result<YnabAccountConfig, Box<dyn Error>> {
        get_saxo_ynab_account_config()
    }
}

impl GetBalance for Mock {
    async fn get(&self) -> Result<f32, Box<dyn Error>> {
        Ok(0.0)
    }
}

impl GetYnabAccountConfig for Saxo {
    async fn get(&self) -> Result<YnabAccountConfig, Box<dyn Error>> {
        get_saxo_ynab_account_config()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AccessTokenResponse {
    access_token: String,
    expires_in: u32,
    refresh_token: String,
    refresh_token_expires_in: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct AccountResponse {
    total_value: f32,
}

impl GetBalance for Saxo {
    async fn get(&self) -> Result<f32, Box<dyn Error>> {
        let config_path = env::var("CONFIG_PATH")?;

        let config = config::Config::builder()
            .add_source(config::File::with_name(&config_path))
            .build()?
            .try_deserialize::<Config>()?;

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;

        let api = pushover::API::new();

        let refreshed_access_token = get_refreshed_access_token(&config, &client, &api).await?;

        let account_response = get_account_value(&client, &refreshed_access_token).await?;

        Ok(account_response.total_value)
    }
}

async fn get_refreshed_access_token(
    config: &Config,
    client: &reqwest::Client,
    api: &pushover::API,
) -> Result<AccessTokenResponse, Box<dyn Error>> {
    let access_token = get_cached_or_live_access_token(&config, &client, &api).await?;

    let refreshed_access_token = refresh_access_token(&config, &client, &access_token).await?;

    std::fs::write(
        config.saxo_access_token_path.clone(),
        serde_json::to_string(&refreshed_access_token)?,
    )?;

    Ok(refreshed_access_token)
}

async fn get_cached_or_live_access_token(
    config: &Config,
    client: &reqwest::Client,
    api: &pushover::API,
) -> Result<AccessTokenResponse, Box<dyn Error>> {
    let valid_refresh_token_o = std::fs::metadata(config.saxo_access_token_path.clone())
        .ok()
        .and_then(|stat| stat.modified().ok())
        .and_then(|modified| {
            let access_token_file = std::fs::read(config.saxo_access_token_path.clone())
                .expect(format!("Unable to read {}", config.saxo_access_token_path).as_str());

            let access_token = serde_json::from_slice::<AccessTokenResponse>(&access_token_file)
                .expect("Unable to parse access_token_file");

            let modified_at = DateTime::<Utc>::from(modified);

            let expires_in = Duration::seconds(access_token.refresh_token_expires_in as i64);

            let expires_at = modified_at.checked_add_signed(expires_in).expect(
                format!(
                    "Unable to add expires_in '{}' to modified_at '{}'",
                    expires_in, modified_at
                )
                .as_str(),
            );

            if Utc::now() > expires_at {
                None
            } else {
                Some(access_token)
            }
        });

    match valid_refresh_token_o {
        Some(valid_refresh_token) => Ok(valid_refresh_token),
        _ => {
            let login_uri = get_login_uri(&config, &client).await?;

            send_login_uri_push_notification(&config, &api, login_uri)?;

            let auth_code = block_until_auth_code()?;

            let access_token = get_access_token(&config, &client, auth_code).await?;

            std::fs::write(
                config.saxo_access_token_path.clone(),
                serde_json::to_string(&access_token)?,
            )?;

            Ok(access_token)
        }
    }
}

fn get_saxo_ynab_account_config() -> Result<YnabAccountConfig, Box<dyn Error>> {
    let config_path = env::var("CONFIG_PATH")?;

    let config = config::Config::builder()
        .add_source(config::File::with_name(&config_path))
        .build()?
        .try_deserialize::<Config>()?;

    let yac = YnabAccountConfig {
        ynab_account_id: config.ynab_saxo_account_id,
    };

    Ok(yac)
}

async fn get_login_uri(
    config: &Config,
    client: &reqwest::Client,
) -> Result<String, Box<dyn Error>> {
    let location = client
        .get(SAXO_AUTH_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .query(&[
            ("response_type", "code"),
            ("client_id", config.saxo_client_id.as_str()),
            ("state", "0"),
            ("redirect_uri", config.saxo_redirect_uri.as_str()),
        ])
        .send()
        .await?
        .headers()
        .get("location")
        .expect("Unable to get Location header")
        .to_str()?
        .to_owned();

    Ok(location)
}

fn block_until_auth_code() -> Result<String, Box<dyn Error>> {
    info!("Waiting for auth code redirect");

    let listener = TcpListener::bind(format!("{}:9999", TAILSCALE_IP))?;

    let (mut stream, _) = listener.accept()?;
    let mut buffer = [0; 512];
    stream.read(&mut buffer).unwrap();

    stream.write("HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nsuccess".as_bytes())?;
    stream.flush()?;

    let mut headers = [httparse::EMPTY_HEADER; 20];
    // TODO: use HTTPS
    let mut req = httparse::Request::new(&mut headers);
    req.parse(&buffer)?;

    let req = reqwest::Url::parse(format!("http://_{}", req.path.unwrap()).as_str())?;

    let code = req
        .query_pairs()
        .find(|s| s.0 == "code")
        .expect("Unable to parse code from redirect_uri")
        .1
        .into_owned();

    Ok(code)
}

fn send_login_uri_push_notification(
    config: &Config,
    api: &pushover::API,
    login_uri: String,
) -> Result<(), Box<dyn Error>> {
    let mut msg = SendMessage::new(
        config.pushover_api_key.clone(),
        config.pushover_user_key.clone(),
        "Login to Saxo",
    );
    msg.set_url(login_uri.clone());
    msg.set_url_title("Login link");

    api.send(&msg)?;

    Ok(())
}

async fn get_access_token(
    config: &Config,
    client: &reqwest::Client,
    code: String,
) -> Result<AccessTokenResponse, Box<dyn Error>> {
    let params = HashMap::from([
        ("client_id", config.saxo_client_id.as_str()),
        ("client_secret", config.saxo_client_secret.as_str()),
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", config.saxo_redirect_uri.as_str()),
    ]);

    let token = client
        .post(SAXO_ACCESS_URL)
        .form(&params)
        .send()
        .await?
        .json::<AccessTokenResponse>()
        .await?;

    Ok(token)
}

async fn refresh_access_token(
    config: &Config,
    client: &reqwest::Client,
    access_token: &AccessTokenResponse,
) -> Result<AccessTokenResponse, Box<dyn Error>> {
    let params = HashMap::from([
        ("client_id", config.saxo_client_id.as_str()),
        ("client_secret", config.saxo_client_secret.as_str()),
        ("grant_type", "refresh_token"),
        ("refresh_token", access_token.refresh_token.as_str()),
        ("redirect_uri", config.saxo_redirect_uri.as_str()),
    ]);

    let token = client
        .post(SAXO_ACCESS_URL)
        .form(&params)
        .send()
        .await?
        .json::<AccessTokenResponse>()
        .await?;

    Ok(token)
}

async fn get_account_value(
    client: &reqwest::Client,
    access_token: &AccessTokenResponse,
) -> Result<AccountResponse, Box<dyn Error>> {
    let resp = client
        .get(format!("{}/port/v1/balances/me", SAXO_API_URL))
        .bearer_auth(access_token.access_token.clone())
        .send()
        .await?
        .json::<AccountResponse>()
        .await?;

    Ok(resp)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    let _saxo = Saxo {};

    let _mock = Mock {};

    update_ynab(_saxo).await
}
