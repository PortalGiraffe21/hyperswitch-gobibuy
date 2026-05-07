mod requests;
mod response;
pub mod transformers;

use std::sync::LazyLock;

use common_enums::enums;
use common_utils::{
    errors::CustomResult,
    ext_traits::XmlExt,
    request::{Method, Request, RequestBuilder, RequestContent},
    types::{AmountConvertor, StringMajorUnit, StringMajorUnitForConnector},
};
use error_stack::{report, ResultExt};
use hyperswitch_domain_models::{
    router_data::{AccessToken, ConnectorAuthType, ErrorResponse, RouterData},
    router_flow_types::{
        access_token_auth::AccessTokenAuth,
        payments::{
            Authorize, Capture, CompleteAuthorize, PSync, PaymentMethodToken, Session,
            SetupMandate, Void,
        },
        refunds::{Execute, RSync},
    },
    router_request_types::{
        AccessTokenRequestData, CompleteAuthorizeData, PaymentMethodTokenizationData,
        PaymentsAuthorizeData, PaymentsCancelData, PaymentsCaptureData, PaymentsSessionData,
        PaymentsSyncData, RefundsData, SetupMandateRequestData,
    },
    router_response_types::{
        ConnectorInfo, PaymentMethodDetails, PaymentsResponseData, RefundsResponseData,
        SupportedPaymentMethods, SupportedPaymentMethodsExt,
    },
    types::{
        PaymentsAuthorizeRouterData, PaymentsCompleteAuthorizeRouterData, PaymentsSyncRouterData,
        RefundExecuteRouterData, RefundSyncRouterData, RefundsRouterData,
    },
};

use hyperswitch_interfaces::{
    api::{
        self, ConnectorCommon, ConnectorCommonExt, ConnectorIntegration, ConnectorSpecifications,
        ConnectorValidation,
    },
    configs::Connectors,
    errors,
    events::connector_api_logs::ConnectorEvent,
    types::{
        PaymentsAuthorizeType, PaymentsCompleteAuthorizeType, PaymentsSyncType, RefundExecuteType,
        RefundSyncType, Response,
    },
    webhooks,
};
use hyperswitch_masking::PeekInterface;
use transformers::{self as nestpay_transformers, NestpayAuthType, NestpayConnectorMeta};

use crate::{
    constants::headers,
    types::ResponseRouterData,
    utils::{self, convert_amount},
};

#[derive(Clone)]
pub struct Nestpay {
    amount_converter: &'static (dyn AmountConvertor<Output = StringMajorUnit> + Sync),
}

impl Nestpay {
    pub fn new() -> &'static Self {
        &Self {
            amount_converter: &StringMajorUnitForConnector,
        }
    }
}

impl<Flow, Request, Response> ConnectorCommonExt<Flow, Request, Response> for Nestpay
where
    Self: ConnectorIntegration<Flow, Request, Response>,
{
    fn build_headers(
        &self,
        _req: &RouterData<Flow, Request, Response>,
        _connectors: &Connectors,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        Ok(vec![(
            headers::CONTENT_TYPE.to_string(),
            self.common_get_content_type().to_string().into(),
        )])
    }
}

impl ConnectorCommon for Nestpay {
    fn id(&self) -> &'static str {
        "nestpay"
    }

    fn get_currency_unit(&self) -> api::CurrencyUnit {
        api::CurrencyUnit::Base
    }

    fn common_get_content_type(&self) -> &'static str {
        "application/xml; charset=UTF-8"
    }

    fn base_url<'a>(&self, connectors: &'a Connectors) -> &'a str {
        connectors.nestpay.base_url.as_ref()
    }

    fn get_auth_header(
        &self,
        _auth_type: &ConnectorAuthType,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        Ok(vec![])
    }

    fn build_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        let response_str = String::from_utf8(res.response.to_vec())
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        event_builder.map(|i| {
            i.set_error(serde_json::json!({
                "error": response_str.clone(),
                "status_code": res.status_code
            }))
        });
        router_env::logger::info!(connector_response=?response_str);

        let parsed = response_str.parse_xml::<response::NestpayCC5Response>();

        Ok(match parsed {
            Ok(cc5) => {
                let code = cc5
                    .proc_return_code
                    .clone()
                    .unwrap_or_else(|| hyperswitch_interfaces::consts::NO_ERROR_CODE.to_string());
                let message = cc5.err_msg.clone().unwrap_or_else(|| {
                    cc5.response
                        .clone()
                        .unwrap_or_else(|| hyperswitch_interfaces::consts::NO_ERROR_MESSAGE.to_string())
                });
                ErrorResponse {
                    code,
                    message: message.clone(),
                    reason: Some(message),
                    status_code: res.status_code,
                    attempt_status: None,
                    connector_transaction_id: cc5.order_id,
                    connector_response_reference_id: None,
                    network_advice_code: None,
                    network_decline_code: None,
                    network_error_message: None,
                    connector_metadata: None,
                }
            }
            Err(_) => ErrorResponse {
                code: hyperswitch_interfaces::consts::NO_ERROR_CODE.to_string(),
                message: response_str.clone(),
                reason: Some(response_str),
                status_code: res.status_code,
                attempt_status: None,
                connector_transaction_id: None,
                connector_response_reference_id: None,
                network_advice_code: None,
                network_decline_code: None,
                network_error_message: None,
                connector_metadata: None,
            },
        })
    }
}

impl ConnectorValidation for Nestpay {
    fn validate_connector_against_payment_request(
        &self,
        capture_method: Option<enums::CaptureMethod>,
        _payment_method: enums::PaymentMethod,
        _pmt: Option<enums::PaymentMethodType>,
    ) -> CustomResult<(), errors::ConnectorError> {
        let capture_method = capture_method.unwrap_or_default();
        match capture_method {
            enums::CaptureMethod::Automatic
            | enums::CaptureMethod::Manual
            | enums::CaptureMethod::SequentialAutomatic => Ok(()),
            _ => Err(errors::ConnectorError::FlowNotSupported {
                flow: capture_method.to_string(),
                connector: self.id().to_string(),
            }
            .into()),
        }
    }
}

impl api::Payment for Nestpay {}
impl api::PaymentSession for Nestpay {}
impl api::ConnectorAccessToken for Nestpay {}
impl api::MandateSetup for Nestpay {}
impl api::PaymentAuthorize for Nestpay {}
impl api::PaymentsCompleteAuthorize for Nestpay {}
impl api::PaymentSync for Nestpay {}
impl api::PaymentCapture for Nestpay {}
impl api::PaymentVoid for Nestpay {}
impl api::Refund for Nestpay {}
impl api::RefundExecute for Nestpay {}
impl api::RefundSync for Nestpay {}
impl api::PaymentToken for Nestpay {}

impl api::ConnectorRedirectResponse for Nestpay {
    fn get_flow_type(
        &self,
        _query_params: &str,
        _json_payload: Option<serde_json::Value>,
        action: enums::PaymentAction,
    ) -> CustomResult<enums::CallConnectorAction, errors::ConnectorError> {
        match action {
            enums::PaymentAction::CompleteAuthorize
            | enums::PaymentAction::PSync
            | enums::PaymentAction::PaymentAuthenticateCompleteAuthorize => {
                Ok(enums::CallConnectorAction::Trigger)
            }
        }
    }
}

impl ConnectorIntegration<PaymentMethodToken, PaymentMethodTokenizationData, PaymentsResponseData>
    for Nestpay
{
}

impl ConnectorIntegration<Session, PaymentsSessionData, PaymentsResponseData> for Nestpay {}

impl ConnectorIntegration<AccessTokenAuth, AccessTokenRequestData, AccessToken> for Nestpay {}

impl ConnectorIntegration<SetupMandate, SetupMandateRequestData, PaymentsResponseData>
    for Nestpay
{
    fn build_request(
        &self,
        _req: &RouterData<SetupMandate, SetupMandateRequestData, PaymentsResponseData>,
        _connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Err(errors::ConnectorError::FlowNotSupported {
            flow: "Setup Mandate".to_string(),
            connector: self.id().to_string(),
        }
        .into())
    }
}

impl ConnectorIntegration<Void, PaymentsCancelData, PaymentsResponseData> for Nestpay {}

impl ConnectorIntegration<Capture, PaymentsCaptureData, PaymentsResponseData> for Nestpay {}

// ── Authorize ─────────────────────────────────────────────────────────────────

impl ConnectorIntegration<Authorize, PaymentsAuthorizeData, PaymentsResponseData> for Nestpay {
    fn get_headers(
        &self,
        req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Ok(format!("{}/fim/api", self.base_url(connectors)))
    }


    fn get_request_body(
        &self,
        req: &PaymentsAuthorizeRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let amount = convert_amount(
            self.amount_converter,
            req.request.minor_amount,
            req.request.currency,
        )?;

        let connector_router_data = requests::NestpayRouterData::from((amount, req));
        let connector_req = requests::NestpayCC5Request::try_from(&connector_router_data)?;

        let xml_bytes = utils::XmlSerializer::serialize_to_xml_bytes(
            &connector_req,
            "1.0",
            Some("UTF-8"),
            None,
            None,
        )
        .change_context(errors::ConnectorError::RequestEncodingFailed)?;

        Ok(RequestContent::RawBytes(xml_bytes))
    }

    fn build_request(
        &self,
        req: &PaymentsAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Post)
                .url(&PaymentsAuthorizeType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(PaymentsAuthorizeType::get_headers(self, req, connectors)?)
                .set_body(PaymentsAuthorizeType::get_request_body(
                    self, req, connectors,
                )?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &PaymentsAuthorizeRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<PaymentsAuthorizeRouterData, errors::ConnectorError> {
        let response_str = String::from_utf8(res.response.to_vec())
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        let response: response::NestpayCC5Response = response_str
            .parse_xml()
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);

        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
        .change_context(errors::ConnectorError::ResponseHandlingFailed)
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

// ── CompleteAuthorize (3DS step 2) ────────────────────────────────────────────

impl ConnectorIntegration<CompleteAuthorize, CompleteAuthorizeData, PaymentsResponseData>
    for Nestpay
{
    fn get_headers(
        &self,
        req: &PaymentsCompleteAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &PaymentsCompleteAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Ok(format!("{}/fim/api", self.base_url(connectors)))
    }

    fn get_request_body(
        &self,
        req: &PaymentsCompleteAuthorizeRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let auth = NestpayAuthType::try_from(&req.connector_auth_type)?;
        let api_username = NestpayConnectorMeta::try_from(&req.connector_meta_data)
            .map(|m| m.api_username)
            .unwrap_or_else(|_| auth.client_id.peek().to_string());
        let mode = if req.test_mode.unwrap_or(true) { "T" } else { "P" };

        let redirect = req
            .request
            .redirect_response
            .as_ref()
            .ok_or(errors::ConnectorError::MissingRequiredField {
                field_name: "redirect_response",
            })?;

        let params = redirect
            .params
            .as_ref()
            .ok_or(errors::ConnectorError::MissingRequiredField {
                field_name: "redirect_response.params",
            })?;

        let parsed: std::collections::HashMap<String, String> =
            serde_urlencoded::from_str(params.peek())
                .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        let pa_res = parsed
            .get("PaRes")
            .ok_or(errors::ConnectorError::MissingRequiredField { field_name: "PaRes" })?
            .clone();
        let md = parsed
            .get("MD")
            .ok_or(errors::ConnectorError::MissingRequiredField { field_name: "MD" })?
            .clone();

        let connector_req = requests::NestpayComplete3dsRequest {
            name: api_username,
            password: auth.api_password,
            client_id: auth.client_id.peek().to_string(),
            request_type: "Auth".to_string(),
            order_id: req
                .request
                .connector_transaction_id
                .clone()
                .unwrap_or_else(|| req.connector_request_reference_id.clone()),
            mode: mode.to_string(),
            pa_res,
            md,
        };

        let xml_bytes = utils::XmlSerializer::serialize_to_xml_bytes(
            &connector_req,
            "1.0",
            Some("UTF-8"),
            None,
            None,
        )
        .change_context(errors::ConnectorError::RequestEncodingFailed)?;

        Ok(RequestContent::RawBytes(xml_bytes))
    }

    fn build_request(
        &self,
        req: &PaymentsCompleteAuthorizeRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Post)
                .url(&PaymentsCompleteAuthorizeType::get_url(
                    self, req, connectors,
                )?)
                .attach_default_headers()
                .headers(PaymentsCompleteAuthorizeType::get_headers(
                    self, req, connectors,
                )?)
                .set_body(PaymentsCompleteAuthorizeType::get_request_body(
                    self, req, connectors,
                )?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &PaymentsCompleteAuthorizeRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<PaymentsCompleteAuthorizeRouterData, errors::ConnectorError> {
        let response_str = String::from_utf8(res.response.to_vec())
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        let response: response::NestpayCC5Response = response_str
            .parse_xml()
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);

        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
        .change_context(errors::ConnectorError::ResponseHandlingFailed)
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

// ── PaymentSync ───────────────────────────────────────────────────────────────

impl ConnectorIntegration<PSync, PaymentsSyncData, PaymentsResponseData> for Nestpay {
    fn get_headers(
        &self,
        req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Ok(format!("{}/fim/api", self.base_url(connectors)))
    }

    fn get_request_body(
        &self,
        req: &PaymentsSyncRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let order_id = req
            .request
            .connector_transaction_id
            .get_connector_transaction_id()
            .change_context(errors::ConnectorError::MissingConnectorTransactionID)?;

        let auth = NestpayAuthType::try_from(&req.connector_auth_type)?;
        let api_username = NestpayConnectorMeta::try_from(&req.connector_meta_data)
            .map(|m| m.api_username)
            .unwrap_or_else(|_| auth.client_id.peek().to_string());
        let mode = if req.test_mode.unwrap_or(true) { "T" } else { "P" };

        let connector_req =
            nestpay_transformers::build_sync_request(&order_id, &auth, &api_username, mode);

        let xml_bytes = utils::XmlSerializer::serialize_to_xml_bytes(
            &connector_req,
            "1.0",
            Some("UTF-8"),
            None,
            None,
        )
        .change_context(errors::ConnectorError::RequestEncodingFailed)?;

        Ok(RequestContent::RawBytes(xml_bytes))
    }

    fn build_request(
        &self,
        req: &PaymentsSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Post)
                .url(&PaymentsSyncType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(PaymentsSyncType::get_headers(self, req, connectors)?)
                .set_body(PaymentsSyncType::get_request_body(self, req, connectors)?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &PaymentsSyncRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<PaymentsSyncRouterData, errors::ConnectorError> {
        let response_str = String::from_utf8(res.response.to_vec())
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        let response: response::NestpayCC5Response = response_str
            .parse_xml()
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);

        RouterData::try_from(ResponseRouterData {
            response,
            data: data.clone(),
            http_code: res.status_code,
        })
        .change_context(errors::ConnectorError::ResponseHandlingFailed)
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

// ── Refund ────────────────────────────────────────────────────────────────────

impl ConnectorIntegration<Execute, RefundsData, RefundsResponseData> for Nestpay {
    fn get_headers(
        &self,
        req: &RefundsRouterData<Execute>,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &RefundsRouterData<Execute>,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Ok(format!("{}/fim/api", self.base_url(connectors)))
    }

    fn get_request_body(
        &self,
        req: &RefundsRouterData<Execute>,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let amount = convert_amount(
            self.amount_converter,
            req.request.minor_refund_amount,
            req.request.currency,
        )?;

        let connector_router_data = requests::NestpayRouterData::from((amount, req));
        let connector_req = requests::NestpayCC5Request::try_from(&connector_router_data)?;

        let xml_bytes = utils::XmlSerializer::serialize_to_xml_bytes(
            &connector_req,
            "1.0",
            Some("UTF-8"),
            None,
            None,
        )
        .change_context(errors::ConnectorError::RequestEncodingFailed)?;

        Ok(RequestContent::RawBytes(xml_bytes))
    }

    fn build_request(
        &self,
        req: &RefundsRouterData<Execute>,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Post)
                .url(&RefundExecuteType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(RefundExecuteType::get_headers(self, req, connectors)?)
                .set_body(RefundExecuteType::get_request_body(self, req, connectors)?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &RefundsRouterData<Execute>,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<RefundsRouterData<Execute>, errors::ConnectorError> {
        let response_str = String::from_utf8(res.response.to_vec())
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        let response: response::NestpayCC5Response = response_str
            .parse_xml()
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);

        let response: RefundsRouterData<Execute> = 
            RefundExecuteRouterData::try_from(crate::types::ResponseRouterData {
                response,
                data: data.clone(),
                http_code: res.status_code,
            })
            .change_context(errors::ConnectorError::ResponseHandlingFailed)?;
        Ok(response)
    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

// ── RefundSync ────────────────────────────────────────────────────────────────

impl ConnectorIntegration<RSync, RefundsData, RefundsResponseData> for Nestpay {
    fn get_headers(
        &self,
        req: &RefundSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Vec<(String, hyperswitch_masking::Maskable<String>)>, errors::ConnectorError>
    {
        self.build_headers(req, connectors)
    }

    fn get_content_type(&self) -> &'static str {
        self.common_get_content_type()
    }

    fn get_url(
        &self,
        _req: &RefundSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<String, errors::ConnectorError> {
        Ok(format!("{}/fim/api", self.base_url(connectors)))
    }

    fn get_request_body(
        &self,
        req: &RefundSyncRouterData,
        _connectors: &Connectors,
    ) -> CustomResult<RequestContent, errors::ConnectorError> {
        let auth = NestpayAuthType::try_from(&req.connector_auth_type)?;
        let api_username = NestpayConnectorMeta::try_from(&req.connector_meta_data)
            .map(|m| m.api_username)
            .unwrap_or_else(|_| auth.client_id.peek().to_string());
        let mode = if req.test_mode.unwrap_or(true) { "T" } else { "P" };
        let order_id = req.request.connector_transaction_id.clone();

        let connector_req =
            nestpay_transformers::build_sync_request(&order_id, &auth, &api_username, mode);

        let xml_bytes = utils::XmlSerializer::serialize_to_xml_bytes(
            &connector_req,
            "1.0",
            Some("UTF-8"),
            None,
            None,
        )
        .change_context(errors::ConnectorError::RequestEncodingFailed)?;

        Ok(RequestContent::RawBytes(xml_bytes))
    }

    fn build_request(
        &self,
        req: &RefundSyncRouterData,
        connectors: &Connectors,
    ) -> CustomResult<Option<Request>, errors::ConnectorError> {
        Ok(Some(
            RequestBuilder::new()
                .method(Method::Post)
                .url(&RefundSyncType::get_url(self, req, connectors)?)
                .attach_default_headers()
                .headers(RefundSyncType::get_headers(self, req, connectors)?)
                .set_body(RefundSyncType::get_request_body(self, req, connectors)?)
                .build(),
        ))
    }

    fn handle_response(
        &self,
        data: &RefundSyncRouterData,
        event_builder: Option<&mut ConnectorEvent>,
        res: Response,
    ) -> CustomResult<RefundSyncRouterData, errors::ConnectorError> {
        let response_str = String::from_utf8(res.response.to_vec())
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        let response: response::NestpayCC5Response = response_str
            .parse_xml()
            .change_context(errors::ConnectorError::ResponseDeserializationFailed)?;

        event_builder.map(|i| i.set_response_body(&response));
        router_env::logger::info!(connector_response=?response);

        let response: RefundSyncRouterData = 
            RefundsRouterData::<RSync>::try_from(crate::types::ResponseRouterData {
                response,
                data: data.clone(),
                http_code: res.status_code,
            })
            .change_context(errors::ConnectorError::ResponseHandlingFailed)?;
        Ok(response)

    }

    fn get_error_response(
        &self,
        res: Response,
        event_builder: Option<&mut ConnectorEvent>,
    ) -> CustomResult<ErrorResponse, errors::ConnectorError> {
        self.build_error_response(res, event_builder)
    }
}

// ── Webhooks ──────────────────────────────────────────────────────────────────

impl webhooks::IncomingWebhook for Nestpay {
    fn get_webhook_object_reference_id(
        &self,
        _request: &webhooks::IncomingWebhookRequestDetails<'_>,
    ) -> CustomResult<api_models::webhooks::ObjectReferenceId, errors::ConnectorError> {
        Err(report!(errors::ConnectorError::WebhooksNotImplemented))
    }

    fn get_webhook_event_type(
        &self,
        _request: &webhooks::IncomingWebhookRequestDetails<'_>,
        _context: Option<&webhooks::WebhookContext>,
    ) -> CustomResult<api_models::webhooks::IncomingWebhookEvent, errors::ConnectorError> {
        Err(report!(errors::ConnectorError::WebhooksNotImplemented))
    }

    fn get_webhook_resource_object(
        &self,
        _request: &webhooks::IncomingWebhookRequestDetails<'_>,
    ) -> CustomResult<Box<dyn hyperswitch_masking::ErasedMaskSerialize>, errors::ConnectorError>
    {
        Err(report!(errors::ConnectorError::WebhooksNotImplemented))
    }
}

// ── ConnectorSpecifications ───────────────────────────────────────────────────

static NESTPAY_SUPPORTED_PAYMENT_METHODS: LazyLock<SupportedPaymentMethods> =
    LazyLock::new(|| {
        let supported_capture_methods = vec![
            enums::CaptureMethod::Automatic,
            enums::CaptureMethod::Manual,
        ];
        let supported_card_networks = vec![
            common_enums::CardNetwork::Visa,
            common_enums::CardNetwork::Mastercard,
            common_enums::CardNetwork::AmericanExpress,
        ];

        let mut supported = SupportedPaymentMethods::new();

        supported.add(
            enums::PaymentMethod::Card,
            enums::PaymentMethodType::Credit,
            PaymentMethodDetails {
                mandates: enums::FeatureStatus::NotSupported,
                refunds: enums::FeatureStatus::Supported,
                supported_capture_methods: supported_capture_methods.clone(),
                specific_features: Some(
                    api_models::feature_matrix::PaymentMethodSpecificFeatures::Card(
                        api_models::feature_matrix::CardSpecificFeatures {
                            three_ds: common_enums::FeatureStatus::Supported,
                            no_three_ds: common_enums::FeatureStatus::Supported,
                            supported_card_networks: supported_card_networks.clone(),
                        },
                    ),
                ),
            },
        );
        supported.add(
            enums::PaymentMethod::Wallet,
            enums::PaymentMethodType::ApplePay,
            PaymentMethodDetails {
                mandates: enums::FeatureStatus::NotSupported,
                refunds: enums::FeatureStatus::NotSupported,
                supported_capture_methods: vec![
                    enums::CaptureMethod::Automatic,
                ],
                specific_features: None,
            },
        );


        supported.add(
            enums::PaymentMethod::Card,
            enums::PaymentMethodType::Debit,
            PaymentMethodDetails {
                mandates: enums::FeatureStatus::NotSupported,
                refunds: enums::FeatureStatus::Supported,
                supported_capture_methods,
                specific_features: Some(
                    api_models::feature_matrix::PaymentMethodSpecificFeatures::Card(
                        api_models::feature_matrix::CardSpecificFeatures {
                            three_ds: common_enums::FeatureStatus::Supported,
                            no_three_ds: common_enums::FeatureStatus::Supported,
                            supported_card_networks,
                        },
                    ),
                ),
            },
        );

        supported
    });

static NESTPAY_CONNECTOR_INFO: ConnectorInfo = ConnectorInfo {
    display_name: "NestPay",
    description:
        "NestPay (Payten/Asseco) is a payment gateway used by Halkbank Macedonia and regional banks.",
    connector_type: enums::HyperswitchConnectorCategory::PaymentGateway,
    integration_status: enums::ConnectorIntegrationStatus::Sandbox,
};

static NESTPAY_SUPPORTED_WEBHOOK_FLOWS: [enums::EventClass; 0] = [];

impl ConnectorSpecifications for Nestpay {
    fn get_connector_about(&self) -> Option<&'static ConnectorInfo> {
        Some(&NESTPAY_CONNECTOR_INFO)
    }

    fn get_supported_payment_methods(&self) -> Option<&'static SupportedPaymentMethods> {
        Some(&*NESTPAY_SUPPORTED_PAYMENT_METHODS)
    }

    fn get_supported_webhook_flows(&self) -> Option<&'static [enums::EventClass]> {
        Some(&NESTPAY_SUPPORTED_WEBHOOK_FLOWS)
    }
}
