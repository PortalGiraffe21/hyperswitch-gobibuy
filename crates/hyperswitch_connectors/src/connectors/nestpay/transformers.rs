use base64::Engine;
use common_enums::enums;
use common_utils::{
    consts::BASE64_ENGINE,
    crypto::{self, GenerateDigest},
    pii,
};
use error_stack::ResultExt;
use hyperswitch_domain_models::{
    payment_method_data,
    router_data::{ConnectorAuthType, ErrorResponse, RouterData},
    router_flow_types::refunds::{Execute, RSync},
    router_request_types::ResponseId,
    router_response_types::{PaymentsResponseData, RefundsResponseData},
    types::{RefundExecuteRouterData, RefundsRouterData},
};
use hyperswitch_interfaces::{
    consts::{NO_ERROR_CODE, NO_ERROR_MESSAGE},
    errors,
};
use hyperswitch_masking::{PeekInterface, Secret};
use serde::{Deserialize, Serialize};

use super::{
    requests::{NestpayBillTo, NestpayCC5Request, NestpayRouterData},
    response::NestpayCC5Response,
};
use crate::{
    types::{RefundsResponseRouterData, ResponseRouterData},
    utils::{CardData, RouterData as _},
};

type Error = error_stack::Report<errors::ConnectorError>;

/// Optional connector metadata stored in the merchant connector account.
#[derive(Debug, Serialize, Deserialize)]
pub struct NestpayConnectorMeta {
    /// API username for the CC5 server-to-server endpoint (the `<Name>` field).
    pub api_username: String,
}

impl TryFrom<&Option<pii::SecretSerdeValue>> for NestpayConnectorMeta {
    type Error = Error;
    fn try_from(meta_data: &Option<pii::SecretSerdeValue>) -> Result<Self, Self::Error> {
        let metadata: Self =
            crate::utils::to_connector_meta_from_secret::<Self>(meta_data.clone())
                .change_context(errors::ConnectorError::InvalidConnectorConfig {
                    config: "metadata",
                })?;
        Ok(metadata)
    }
}

pub struct NestpayAuthType {
    pub client_id: Secret<String>,
    /// Only used for HASH computation — never sent in plain requests.
    pub store_key: Secret<String>,
    pub api_password: Secret<String>,
}

impl TryFrom<&ConnectorAuthType> for NestpayAuthType {
    type Error = Error;
    fn try_from(auth_type: &ConnectorAuthType) -> Result<Self, Self::Error> {
        match auth_type {
            ConnectorAuthType::SignatureKey {
                api_key,
                key1,
                api_secret,
            } => Ok(Self {
                client_id: api_key.to_owned(),
                store_key: key1.to_owned(),
                api_password: api_secret.to_owned(),
            }),
            _ => Err(errors::ConnectorError::FailedToObtainAuthType.into()),
        }
    }
}

/// Compute the NestPay HASH ver3 signature.
///
/// 1. Sort param names A→Z (case-sensitive)
/// 2. Escape `\` → `\\` and `|` → `\|` in each value
/// 3. Join values with `|`
/// 4. Append `|<store_key>`
/// 5. Return `Base64(SHA-512(plaintext))`
pub fn compute_nestpay_hash(
    params: &[(&str, &str)],
    store_key: &str,
) -> Result<String, Error> {
    let mut sorted = params.to_vec();
    sorted.sort_by_key(|(k, _)| *k);

    let escaped_values: Vec<String> = sorted
        .iter()
        .map(|(_, v)| v.replace('\\', "\\\\").replace('|', "\\|"))
        .collect();

    let plaintext = format!("{}|{}", escaped_values.join("|"), store_key);

    let hash_bytes = crypto::Sha512
        .generate_digest(plaintext.as_bytes())
        .change_context(errors::ConnectorError::RequestEncodingFailed)
        .attach_printable("SHA-512 digest failed")?;

    Ok(BASE64_ENGINE.encode(hash_bytes))
}

impl TryFrom<&NestpayRouterData<&hyperswitch_domain_models::types::PaymentsAuthorizeRouterData>>
    for NestpayCC5Request
{
    type Error = Error;

    fn try_from(
        item: &NestpayRouterData<&hyperswitch_domain_models::types::PaymentsAuthorizeRouterData>,
    ) -> Result<Self, Self::Error> {
        let auth = NestpayAuthType::try_from(&item.router_data.connector_auth_type)?;

        let api_username = NestpayConnectorMeta::try_from(&item.router_data.connector_meta_data)
            .map(|m| m.api_username)
            .unwrap_or_else(|_| auth.client_id.peek().to_string());

        let (card_number, expires, cvv2val, bill_name) =
            match &item.router_data.request.payment_method_data {
                payment_method_data::PaymentMethodData::Card(card) => {
                    let exp = format!(
                        "{}/{}",
                        card.card_exp_month.peek(),
                        card.get_card_expiry_year_2_digit()?.peek()
                    );
                    let name = item
                        .router_data
                        .get_optional_billing_full_name()
                        .map(|s| s.peek().clone())
                        .unwrap_or_default();
                    (
                        Some(card.card_number.clone()),
                        Some(Secret::new(exp)),
                        Some(card.card_cvc.clone()),
                        name,
                    )
                }
                payment_method_data::PaymentMethodData::Wallet(
                    payment_method_data::WalletData::ApplePay(_)  // fix 1
                ) => {
                    return Err(errors::ConnectorError::NotImplemented(
                        "Apple Pay must be decrypted to card via network_tokenization before reaching NestPay".to_string(),
                    )
                    .into())
                }
                _ => {
                    return Err(errors::ConnectorError::NotImplemented(
                        "Payment method not supported by NestPay".to_string(),
                    )
                    .into())
                }
            };

        let mode = if item.router_data.test_mode.unwrap_or(true) {
            "T"
        } else {
            "P"
        };

        Ok(Self {
            name: api_username,
            password: auth.api_password,
            client_id: auth.client_id.peek().to_string(),
            request_type: "Auth".to_string(),
            order_id: item.router_data.connector_request_reference_id.clone(),
            group_id: String::new(),
            trans_id: String::new(),
            user_id: String::new(),
            total: Some(item.amount.clone()),
            currency: Some(
                item.router_data
                    .request
                    .currency
                    .iso_4217()
                    .to_string(),
            ),
            number: card_number,
            expires,
            cvv2val,
            mode: mode.to_string(),
            bill_to: if bill_name.is_empty() {
                None
            } else {
                Some(NestpayBillTo { name: bill_name })
            },
        })
    }
}

impl<F> TryFrom<&NestpayRouterData<&hyperswitch_domain_models::types::RefundsRouterData<F>>>
    for NestpayCC5Request
{
    type Error = Error;

    fn try_from(
        item: &NestpayRouterData<&hyperswitch_domain_models::types::RefundsRouterData<F>>,
    ) -> Result<Self, Self::Error> {
        let auth = NestpayAuthType::try_from(&item.router_data.connector_auth_type)?;

        let api_username = NestpayConnectorMeta::try_from(&item.router_data.connector_meta_data)
            .map(|m| m.api_username)
            .unwrap_or_else(|_| auth.client_id.peek().to_string());

        let mode = if item.router_data.test_mode.unwrap_or(true) {
            "T"
        } else {
            "P"
        };

        Ok(Self {
            name: api_username,
            password: auth.api_password,
            client_id: auth.client_id.peek().to_string(),
            request_type: "Credit".to_string(),
            order_id: item.router_data.request.connector_transaction_id.clone(),
            group_id: String::new(),
            trans_id: String::new(),
            user_id: String::new(),
            total: Some(item.amount.clone()),
            currency: Some(
                item.router_data
                    .request
                    .currency
                    .iso_4217()
                    .to_string(),
            ),
            number: None,
            expires: None,
            cvv2val: None,
            mode: mode.to_string(),
            bill_to: None,
        })
    }
}

/// Build a sync-query CC5Request (no card details, queries existing order status).
pub fn build_sync_request(
    order_id: &str,
    auth: &NestpayAuthType,
    api_username: &str,
    mode: &str,
) -> NestpayCC5Request {
    NestpayCC5Request {
        name: api_username.to_string(),
        password: auth.api_password.clone(),
        client_id: auth.client_id.peek().to_string(),
        request_type: "PostAuth".to_string(),
        order_id: order_id.to_string(),
        group_id: String::new(),
        trans_id: String::new(),
        user_id: String::new(),
        total: None,
        currency: None,
        number: None,
        expires: None,
        cvv2val: None,
        mode: mode.to_string(),
        bill_to: None,
    }
}

fn nestpay_attempt_status(response: &NestpayCC5Response) -> enums::AttemptStatus {
    if response.pa_req.is_some() && response.acs_url.is_some() {
        return enums::AttemptStatus::AuthenticationPending;
    }
    match response.response.as_deref() {
        Some("Approved") => enums::AttemptStatus::Charged,
        _ => enums::AttemptStatus::Failure,
    }
}

fn nestpay_refund_status(response: &NestpayCC5Response) -> enums::RefundStatus {
    match response.response.as_deref() {
        Some("Approved") => enums::RefundStatus::Success,
        _ => enums::RefundStatus::Failure,
    }
}

fn build_error_response(response: &NestpayCC5Response, status_code: u16) -> ErrorResponse {
    let code = response
        .proc_return_code
        .clone()
        .unwrap_or_else(|| NO_ERROR_CODE.to_string());
    let message = response.err_msg.clone().unwrap_or_else(|| {
        response
            .response
            .clone()
            .unwrap_or_else(|| NO_ERROR_MESSAGE.to_string())
    });
    ErrorResponse {
        code,
        message: message.clone(),
        reason: Some(message),
        status_code,
        attempt_status: None,
        connector_transaction_id: response.order_id.clone(),
        connector_response_reference_id: None,
        network_advice_code: None,
        network_decline_code: None,
        network_error_message: None,
        connector_metadata: None,
    }
}

impl<F, T>
    TryFrom<ResponseRouterData<F, NestpayCC5Response, T, PaymentsResponseData>>
    for RouterData<F, T, PaymentsResponseData>
{
    type Error = Error;

    fn try_from(
        item: ResponseRouterData<F, NestpayCC5Response, T, PaymentsResponseData>,
    ) -> Result<Self, Self::Error> {
        let status = nestpay_attempt_status(&item.response);
        let redirection_data = match (
            &item.response.acs_url,
            &item.response.pa_req,
            &item.response.md,
        ) {
            (Some(acs_url), Some(pa_req), md) => {
                let mut form_fields = std::collections::HashMap::new();
                form_fields.insert("PaReq".to_string(), pa_req.clone());
                form_fields.insert("TermUrl".to_string(), 
                item.data.return_url.clone().unwrap_or_default());
                if let Some(md_val) = md {
                    form_fields.insert("MD".to_string(), md_val.clone());
                }
                Some(hyperswitch_domain_models::router_response_types::RedirectForm::Form {
                    endpoint: acs_url.clone(),
                    method: common_utils::request::Method::Post,
                    form_fields,
                })
            }
            _ => None,
        };


        let order_id = item
            .response
            .order_id
            .clone()
            .unwrap_or_else(|| item.data.connector_request_reference_id.clone());

        let response = if status == enums::AttemptStatus::Failure {
            Err(build_error_response(&item.response, item.http_code))
        } else {
            Ok(PaymentsResponseData::TransactionResponse {
                resource_id: ResponseId::ConnectorTransactionId(order_id.clone()),
                redirection_data: Box::new(redirection_data),
                mandate_reference: Box::new(None),
                connector_metadata: None,
                network_txn_id: None,
                connector_response_reference_id: Some(order_id),
                incremental_authorization_allowed: None,
                authentication_data: None,
                charges: None,
            })
        };

        Ok(Self {
            status,
            response,
            ..item.data
        })
    }
}

impl TryFrom<RefundsResponseRouterData<Execute, NestpayCC5Response>> for RefundExecuteRouterData {
    type Error = Error;

    fn try_from(
        item: RefundsResponseRouterData<Execute, NestpayCC5Response>,
    ) -> Result<Self, Self::Error> {
        let refund_status = nestpay_refund_status(&item.response);
        let connector_refund_id = item
            .response
            .order_id
            .clone()
            .or_else(|| item.response.trans_id.clone())
            .unwrap_or_default();

        Ok(Self {
            response: Ok(RefundsResponseData {
                connector_refund_id,
                refund_status,
            }),
            ..item.data
        })
    }
}

impl TryFrom<RefundsResponseRouterData<RSync, NestpayCC5Response>>
    for RefundsRouterData<RSync>
{
    type Error = Error;

    fn try_from(
        item: RefundsResponseRouterData<RSync, NestpayCC5Response>,
    ) -> Result<Self, Self::Error> {
        let refund_status = nestpay_refund_status(&item.response);
        let connector_refund_id = item
            .response
            .order_id
            .clone()
            .or_else(|| item.response.trans_id.clone())
            .unwrap_or_default();

        Ok(Self {
            response: Ok(RefundsResponseData {
                connector_refund_id,
                refund_status,
            }),
            ..item.data
        })
    }
}
