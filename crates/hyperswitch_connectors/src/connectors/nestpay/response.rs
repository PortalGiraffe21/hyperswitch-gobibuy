use serde::{Deserialize, Serialize};

/// CC5Response XML returned by the NestPay server-to-server API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename = "CC5Response")]
pub struct NestpayCC5Response {
    #[serde(rename = "OrderId")]
    pub order_id: Option<String>,
    #[serde(rename = "GroupId")]
    pub group_id: Option<String>,
    /// "Approved", "Decline", or "Error"
    #[serde(rename = "Response")]
    pub response: Option<String>,
    #[serde(rename = "AuthCode")]
    pub auth_code: Option<String>,
    #[serde(rename = "HostRefNum")]
    pub host_ref_num: Option<String>,
    /// "00" means success
    #[serde(rename = "ProcReturnCode")]
    pub proc_return_code: Option<String>,
    #[serde(rename = "TransId")]
    pub trans_id: Option<String>,
    #[serde(rename = "ErrMsg")]
    pub err_msg: Option<String>,
    #[serde(rename = "PaReq")]
    pub pa_req: Option<String>,
    #[serde(rename = "ACSUrl")]
    pub acs_url: Option<String>,
    #[serde(rename = "MD")]
    pub md: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NestpayErrorResponse {
    pub code: String,
    pub message: String,
    pub reason: Option<String>,
}
