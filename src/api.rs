use std::{fmt::Display, str::FromStr};

use cookie::Cookie;
use lazy_static::lazy_static;
use regex::Regex;
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

    pub async fn calculate_next_step(&self, phone_number: String) -> Result<NextStep, ApiError> {
        let res: BiedApiResponce<NextStepResponce> = self
            .api_rq(
                "CMA_Onboarding_MCW/PhoneNumberFlow/PhoneRegistrationMain/ActionCalculateNextStep",
                &self.config.next_step_version,
                self.get_anon_auth(),
                &NextStepRequest { phone_number },
            )?
            .send()
            .await?
            .json()
            .await?;

        match res.data.next_step.as_str() {
            "NewAccount" => Ok(NextStep::NewAccount),
            "AccountExist" | "Login" => Ok(NextStep::AccountExist),
            _ => Err(ApiError(format!(
                "Unknown next step - {}",
                res.data.next_step
            ))),
        }
    }

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

        // TODO: don't clone needlessly
        Ok(res
            .data
            .j4y
            .list
            .into_iter()
            .map(|e| e.into())
            .filter(|e: &Offer| !e.regular_price_unit.is_empty())
            .collect())
    }

    pub async fn login(
        &self,
        phone_number: String,
        sms_code: String,
    ) -> Result<AuthenticatedUser, ApiError> {
        let res = self
            .api_rq(
                &format!(
                    "{}/RegistrationFlow/OnBoarding/ActionCMA_Login",
                    self.config.brand_name
                ),
                &self.config.login_api_version,
                self.get_anon_auth(),
                &LoginRequest {
                    pin_code: sms_code,
                    phone_number: phone_number.clone(),
                },
            )?
            .send()
            .await?;

        let mut u1 = None;
        let mut u2 = None;

        for e in res.headers().get_all("set-cookie") {
            let val = e
                .to_str()
                .map_err(|_| ApiError("Header parsing error".to_string()))?;
            if val.starts_with("nr1Users") {
                u1 = Some(val.to_string());
            } else if val.starts_with("nr2Users") {
                u2 = Some(val.to_string());
            }
        }

        let users1 = Cookie::from_str(&u1.ok_or(ApiError("".to_string()))?)?;
        let users2 = Cookie::from_str(&u2.ok_or(ApiError("".to_string()))?)?;

        let body: BiedApiResponce<LoginResponce> = res.json().await?;

        let csrf_token = extract_csrf_token(
            &percent_encoding::percent_decode_str(users2.value()).decode_utf8_lossy(),
        )
        .ok_or(ApiError("can't find the csrf token".to_string()))?;

        Ok(AuthenticatedUser {
            auth_token: body.data.customer_token,
            card_number: body.data.new_customer.card_number,
            external_id: body.data.new_customer.customer_external_id,
            phone_number,
            auth: AuthData {
                users1: users1.value().to_string(),
                users2: users2.value().to_string(),
                csrf_token,
            },
        })
    }

    pub fn new(config: ApiConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    pub async fn register(
        &self,
        phone_number: String,
        sms_code: String,
        first_name: String,
    ) -> Result<(), ApiError> {
        self.api_rq(
            "CMA_Onboarding_MCW/NewUserFlow/NewAccountMain/ActionCreateNewUserAndAcceptTerms",
            &self.config.create_account_version,
            self.get_anon_auth(),
            &RegisterRequest {
                accepted_legal_document_id_list: BiedListWrapper {
                    list: self.config.legal_ids.clone(),
                },
                customer: CustomerData::new(first_name, phone_number),
                deafult_locale: "en-001".to_string(),
                deafult_store_id: "100".to_string(),
                pin_code: sms_code,
            },
        )?
        .send()
        .await?
        .json::<BiedApiResponce<RegisterResponce>>()
        .await?;

        Ok(())
    }

    pub async fn send_sms_code(&self, phone_number: String) -> Result<(), ApiError> {
        let res: BiedApiResponce<SmsVerificationResponce> = self
            .api_rq(
                "CMA_Onboarding_MCW/ActionSendSMSAndRegistration_withValidation_New",
                &self.config.sms_api_version,
                self.get_anon_auth(),
                &SmsVerificationRequest {
                    phone_number,
                    inserted_pin_code: "".to_string(),
                },
            )?
            .send()
            .await?
            .json()
            .await?;

        if res.data.blocked_in_minutes > 0 {
            Err(ApiError(format!(
                "This number is blocked for {} minutes!",
                res.data.blocked_in_minutes
            )))
        } else if res.data.is_blocked {
            Err(ApiError("This number is blocked!".to_string()))
        } else if !res.data.is_sms_sent {
            Err(ApiError(format!(
                "SMS not sent! Error: {}",
                res.data.error_message
            )))
        } else {
            Ok(())
        }
    }
    fn get_anon_auth(&self) -> AuthData {
        AuthData {
            users1: "".to_string(),
            users2: "".to_string(),
            csrf_token: self.config.anonymous_csrf.clone(),
        }
    }
}

fn extract_csrf_token(users2: &str) -> Option<String> {
    lazy_static! {
        static ref RE: Regex = Regex::new(r"crf=([^;]+)").unwrap();
    }
    Some(
        RE.captures(users2)?
            .get(0)?
            .as_str()
            .to_string()
            .chars()
            .skip(4)
            .collect(),
    )
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
                .filter(|e| e.is_empty())
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
    name: String,
    details: String,
    limit: String,
    image: Option<String>,
    human_time: String,
    regular_price: String,
    regular_price_unit: String,
    offer_price: String,
    offer_price_unit: String,
    discount_percent: i32,
}

impl Display for AuthenticatedUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "phone: `{}`; exid: `{}`; card: `{}`;",
            self.phone_number, self.external_id, self.card_number
        )
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AuthenticatedUser {
    pub phone_number: String,
    auth_token: String,
    pub card_number: String,
    external_id: String,
    pub auth: AuthData,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct AuthData {
    users1: String,
    users2: String,
    csrf_token: String,
}

#[derive(Debug)]
pub enum NextStep {
    NewAccount,
    AccountExist,
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
#[serde(rename_all = "PascalCase")]
struct LoginRequest {
    pin_code: String,
    phone_number: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct SmsVerificationRequest {
    phone_number: String,
    inserted_pin_code: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct NextStepRequest {
    phone_number: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct RegisterRequest {
    accepted_legal_document_id_list: BiedListWrapper<String>,
    customer: CustomerData,
    deafult_locale: String,
    deafult_store_id: String,
    pin_code: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct CustomerData {
    curd_number: String,
    date_of_birth: String,
    email: String,
    first_name: String,
    guid: String,
    last_name: String,
    phone_number: String,
    user_id: i32,
}

impl CustomerData {
    fn new(first_name: String, phone_number: String) -> Self {
        Self {
            curd_number: "".to_string(),
            date_of_birth: "1900-01-01".to_string(),
            email: "".to_string(),
            first_name,
            guid: "".to_string(),
            last_name: "".to_string(),
            phone_number,
            user_id: 0,
        }
    }
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
struct RegisterResponce {
    customer_id: String,
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
#[serde(rename_all = "PascalCase")]
struct SmsVerificationResponce {
    error_message: String,
    #[serde(rename = "IsSMSSent")]
    is_sms_sent: bool,
    is_blocked: bool,
    blocked_in_minutes: i32,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct NextStepResponce {
    next_step: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LoginResponce {
    new_customer: NewCustomerData,
    customer_token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct NewCustomerData {
    card_number: String,
    customer_external_id: String,
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
