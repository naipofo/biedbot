use serde::{Deserialize, Serialize};
use std::fs;

pub fn get_secrets() -> Secrets {
    toml::from_str(&fs::read_to_string("secrets.toml").unwrap()).unwrap()
}

#[derive(Deserialize, Serialize)]
pub struct Secrets {
    pub telegram_config: TelegramConfig,
    pub api_config: ApiConfig,
    pub ean_frontend: String,
    pub cdn_root: String,
}

#[derive(Deserialize, Serialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub maintainer_ids: Vec<u64>,
}

#[derive(Deserialize, Serialize)]
pub struct ApiConfig {
    pub api_root: String,
    pub brand_name: String,
    pub anonymous_csrf: String,
    pub legal_ids: Vec<String>,
    pub module_version: String,
    pub sms_api_version: String,
    pub next_step_version: String,
    pub create_account_version: String,
    pub login_api_version: String,
    pub promo_sync_api_version: String,
}
