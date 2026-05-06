use cards::CardNumber;
use common_utils::types::StringMajorUnit;
use hyperswitch_masking::Secret;
use serde::{Deserialize, Serialize};

pub struct NestpayRouterData<T> {
    pub amount: StringMajorUnit,
    pub router_data: T,
}

impl<T> From<(StringMajorUnit, T)> for NestpayRouterData<T> {
    fn from((amount, item): (StringMajorUnit, T)) -> Self {
        Self {
            amount,
            router_data: item,
        }
    }
}

/// NestPay CC5Request XML — sent to the server-to-server API endpoint.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename = "CC5Request")]
pub struct NestpayCC5Request {
    /// API username (Name field in NestPay docs)
    #[serde(rename = "Name")]
    pub name: String,
    /// API password
    #[serde(rename = "Password")]
    pub password: Secret<String>,
    /// Merchant client ID
    #[serde(rename = "ClientId")]
    pub client_id: String,
    /// Transaction type: Auth, PostAuth, Credit, Void
    #[serde(rename = "Type")]
    pub request_type: String,
    /// Merchant order identifier
    #[serde(rename = "OrderId")]
    pub order_id: String,
    #[serde(rename = "GroupId")]
    pub group_id: String,
    #[serde(rename = "TransId")]
    pub trans_id: String,
    #[serde(rename = "UserId")]
    pub user_id: String,
    /// Decimal amount string, e.g. "10.00"
    #[serde(skip_serializing_if = "Option::is_none", rename = "Total")]
    pub total: Option<StringMajorUnit>,
    /// ISO 4217 numeric currency code, e.g. "807" for MKD
    #[serde(skip_serializing_if = "Option::is_none", rename = "Currency")]
    pub currency: Option<String>,
    /// Card PAN
    #[serde(skip_serializing_if = "Option::is_none", rename = "Number")]
    pub number: Option<CardNumber>,
    /// Card expiry in MM/YY format
    #[serde(skip_serializing_if = "Option::is_none", rename = "Expires")]
    pub expires: Option<Secret<String>>,
    /// CVV/CVC
    #[serde(skip_serializing_if = "Option::is_none", rename = "Cvv2Val")]
    pub cvv2val: Option<Secret<String>>,
    /// "P" for production, "T" for test
    #[serde(rename = "Mode")]
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none", rename = "BillTo")]
    pub bill_to: Option<NestpayBillTo>,
    /// "3d_pay" for Apple Pay S2S, "3d_pay_hosting" for hosted-3DS card redirect
    #[serde(skip_serializing_if = "Option::is_none", rename = "Storetype")]
    pub storetype: Option<String>,
    /// Apple Pay / 3DS CAVV cryptogram (Base64)
    #[serde(skip_serializing_if = "Option::is_none", rename = "PayerAuthenticationCode")]
    pub payer_authentication_code: Option<Secret<String>>,
    /// ECI value from Apple Pay token (e.g. "05" Visa, "02" Mastercard)
    #[serde(skip_serializing_if = "Option::is_none", rename = "Eci")]
    pub eci: Option<String>,
    /// 3DS authentication status indicator — "Y" signals successful authentication
    #[serde(skip_serializing_if = "Option::is_none", rename = "Md")]
    pub md: Option<String>,
    /// Apple Pay transaction identifier (XID)
    #[serde(skip_serializing_if = "Option::is_none", rename = "PayerTxnId")]
    pub payer_txn_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NestpayBillTo {
    #[serde(rename = "Name")]
    pub name: String,
}
