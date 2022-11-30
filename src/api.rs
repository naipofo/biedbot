use std::fmt::Display;

use reqwest::{header, Client, RequestBuilder};
use serde::{Deserialize, Serialize};

use crate::secrets::ApiConfig;

pub struct BiedApi {
    config: ApiConfig,
    client: Client,
}

impl BiedApi {
    fn api_rq<T>(
        &self,
        url: &str,
        api_version: &str,
        auth: AuthData,
        data: &T,
    ) -> Result<RequestBuilder, ApiError>
    where
        T: ?Sized + Serialize,
    {
        Ok(self
            .client
            .post(format!("{}{}", self.config.api_root, url))
            .body(serde_json::to_string(&BiedApiRequest {
                version_info: RequestVersionInfo {
                    module_version: self.config.module_version.to_string(),
                    api_version: api_version.to_string(),
                },
                view_name: "RegistrationFlow.OnBoarding".to_string(),
                input_parameters: data,
            })?)
            .header(header::CONTENT_TYPE, "application/json; charset=UTF-8")
            .header("x-csrftoken", auth.csrf_token)
            .header(
                "cookie",
                format!("nr1Users={}; nr2Users={};", auth.users1, auth.users2),
            ))
    }

    //TODO: Allow for image only offers
    pub async fn get_offers(&self, auth: AuthData) -> Result<Vec<Offer>, ApiError> {
        let res: BiedApiResponce<OfferResponce> = self
            .api_rq(
                &format!("{}_Sync/ActionServerDataSync_2_J4y", self.config.brand_name),
                &self.config.promo_sync_api_version,
                auth,
                &OfferRequest {
                    j4y_cache_refresh: "2022-01-01T10:10:10.101Z".to_string(),
                },
            )?
            .send()
            .await?
            .json()
            .await?;

        Ok(res
            .data
            .j4y
            .list
            .into_iter()
            .map(|e| e.into())
            .filter(|e: &Offer| !e.name.is_empty())
            .collect())
    }

    pub fn new(config: ApiConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Debug)]
pub struct ApiError(String);

impl From<reqwest::Error> for ApiError {
    fn from(e: reqwest::Error) -> Self {
        ApiError(format!("{:?}", e))
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError(format!("{:?}", e))
    }
}

impl From<cookie::ParseError> for ApiError {
    fn from(e: cookie::ParseError) -> Self {
        ApiError(format!("{:?}", e))
    }
}

impl Into<Offer> for OfferElement {
    fn into(self) -> Offer {
        Offer {
            id: self.offer_id_ext,
            name: self.name,
            details: format!(
                "{};{}\n{};{}",
                self.description, self.promo_details, self.tag_top_line, self.tag_bottom_line
            ),
            limit: self.limits,
            image: vec![self.full_screen_image_url, self.image_url, self.thumb_url]
                .into_iter()
                .filter(|e| !e.is_empty())
                .collect::<Vec<_>>()
                .first()
                .map(|e| e.to_string()),
            human_time: self.promotion_time,
            regular_price: self.regular_price,
            regular_price_unit: self.regular_price_per_unit,
            offer_price: self.promo_price,
            offer_price_unit: self.price_per_unit,
            discount_percent: self.discount,
        }
    }
}

impl Display for Offer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\n{}\n{} -> {}\n{} -> {}\n{}",
            self.name,
            self.details,
            self.regular_price,
            self.offer_price,
            self.regular_price_unit,
            self.offer_price_unit,
            self.limit
        )
    }
}

impl Offer {
    pub fn short_display(&self) -> String {
        format!(
            "{} - {} => {}",
            self.name, self.regular_price_unit, self.offer_price_unit
        )
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Offer {
    id: String,
    pub name: String,
    pub details: String,
    pub limit: String,
    pub image: Option<String>,
    pub human_time: String,
    pub regular_price: String,
    pub regular_price_unit: String,
    pub offer_price: String,
    pub offer_price_unit: String,
    pub discount_percent: i32,
}

impl Display for AuthenticatedUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "phone: `{}`; card: `{}`;",
            self.phone_number, self.card_number
        )
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AuthenticatedUser {
    pub phone_number: String,
    pub card_number: String,
    pub auth: AuthData,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AuthData {
    pub users1: String,
    pub users2: String,
    pub csrf_token: String,
}

#[derive(Deserialize)]
struct OfferResponce {
    #[serde(rename = "J4y")]
    j4y: BiedListWrapper<OfferElement>,
}

#[derive(Serialize)]
struct OfferRequest {
    #[serde(rename = "J4yCacheRefresh")]
    j4y_cache_refresh: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BiedApiRequest<T> {
    version_info: RequestVersionInfo,
    view_name: String,
    input_parameters: T,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestVersionInfo {
    module_version: String,
    api_version: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct OfferElement {
    offer_type: String,
    offer_id_ext: String,
    name: String,
    promotion_time: String,
    description: String,
    promo_price: String,
    regular_price: String,
    discount: i32,
    tag_top_line: String,
    tag_bottom_line: String,
    promo_details: String,
    price_per_unit: String,
    limits: String,
    regular_price_per_unit: String,
    product_url: String,
    #[serde(rename = "ThumbURL")]
    thumb_url: String,
    #[serde(rename = "ImageURL")]
    image_url: String,
    #[serde(rename = "FullScreenImageURL")]
    full_screen_image_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionInfoResponce {
    has_module_version_changed: bool,
    has_api_version_changed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BiedApiResponce<T> {
    version_info: VersionInfoResponce,
    data: T,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "PascalCase")]
struct BiedListWrapper<T> {
    list: Vec<T>,
}
