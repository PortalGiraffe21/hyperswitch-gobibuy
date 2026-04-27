Before implementing, read these files in full:
- add_connector.md
- crates/hyperswitch_connectors/src/connectors/globalpay/requests.rs
- crates/hyperswitch_connectors/src/connectors/globalpay/response.rs
- crates/hyperswitch_connectors/src/connectors/globalpay/transformers.rs
- crates/hyperswitch_connectors/src/connectors.rs
- crates/hyperswitch_connectors/src/lib.rs

Then implement the following:

# NestPay Connector Implementation

You are adding a new payment connector called `nestpay` to the Hyperswitch monorepo.

## Location
`crates/hyperswitch_connectors/src/connectors/nestpay/`

## Reference Connector
Use `crates/hyperswitch_connectors/src/connectors/globalpay/` as your structural reference.
It has three files: `requests.rs`, `response.rs`, `transformers.rs`.
Follow the same pattern for nestpay.

## Files to create
1. `crates/hyperswitch_connectors/src/connectors/nestpay/requests.rs`
2. `crates/hyperswitch_connectors/src/connectors/nestpay/response.rs`
3. `crates/hyperswitch_connectors/src/connectors/nestpay/transformers.rs`
4. `crates/hyperswitch_connectors/src/connectors/nestpay.rs` (the main connector impl)

## Files to modify
1. `crates/hyperswitch_connectors/src/connectors.rs` — add `pub mod nestpay;`
2. `crates/hyperswitch_connectors/src/lib.rs` — register connector if needed
3. Any enums file that lists all connectors (search for where `globalpay` is registered as an enum variant and add `Nestpay` in the same pattern)

---

## Connector Details

### Gateway Info
- Name: `nestpay`
- Provider: Payten / NestPay (Halkbank Macedonia)
- 3D Gate URL: `https://torus-stage-halkbankmacedonia.asseco-see.com.tr/fim/est3Dgate`
- API Post URL: `https://torus-stage-halkbankmacedonia.asseco-see.com.tr/fim/api`
- Encoding: UTF-8 (always send `<input type="hidden" name="encoding" value="UTF-8">`)

### Auth Type
Three fields stored in ConnectorAuthType:
- `client_id` (called `clientid` in requests) — e.g. `180000100`
- `store_key` (called `storeKey` in hash, never sent as plain param) — e.g. `TEST1234`
- `api_password` (used for server-to-server API calls) — e.g. `TEmp.291910@`

### Supported Store Types
- `3D` — standard 3D Secure
- `3D_PAY` — 3D Pay
- `3D_PAY_HOSTING` — 3D Pay Hosting

### Currency
- Default: `807` (MKD - Macedonian Denar)
- Amount format: decimal string e.g. `"95.93"`

---

## HASH VERSION 3 ALGORITHM — implement exactly as described

### For REQUESTS:
1. Collect ALL request parameters being posted EXCEPT `storeKey`, `encoding`, `hash`
2. Sort parameter names alphabetically A→Z (case-sensitive)
3. For each sorted parameter, take its value
4. Escape `\` as `\\` and `|` as `\|` in each value
5. Join all escaped values with `|`
6. Append `|` + storeKey (raw, unescaped) at the very end
7. Hash: `Base64(SHA512(plaintext))` using SHA-512
8. Send result as `hash` parameter, and always send `hashAlgorithm=ver3`

### For RESPONSES (verification):
- NestPay returns a `HASH` parameter in the response
- To verify: collect all response parameters EXCEPT `encoding`, `countdown`, `hash`/`HASH`
- Sort alphabetically, join with `|`, append `|storeKey`
- Compute `Base64(SHA512(...))` and compare with returned `HASH`

### Important notes:
- Parameters `encoding` and `hash` are ALWAYS ignored in hash calculation
- Parameter `countdown` is ALSO ignored in response hash calculation
- Empty values ARE included (e.g. `Instalment=` → include empty string in hash)
- Use `common_utils::crypto` for SHA512 and base64 — look at how globalpay transformers.rs imports and uses `crypto::GenerateDigest`

### Example plaintext for request:
95.93|billToCompany|name|https://callback|100200127|949|https://failurl|ver3||tr|https://okurl|5|87954458746|3D|Auth|TEST1234
Hash = Base64(SHA512(above plaintext))

---

## Request Parameters (3D form post)

These are sent as HTML form POST to the 3D Gate URL:

| Parameter | Value |
|---|---|
| clientid | connector auth client_id |
| storetype | `3D_PAY` (or `3D` / `3D_PAY_HOSTING`) |
| amount | decimal string e.g. `"10.00"` |
| currency | `807` |
| okurl | return URL on success (must return HTTP 200, no redirects) |
| failUrl | return URL on failure (must return HTTP 200, no redirects) |
| callbackUrl | callback URL |
| TranType | `Auth` for authorization |
| instalment | installment count, empty string if none |
| rnd | random nonce string |
| lang | `en` |
| hashAlgorithm | `ver3` |
| hash | computed hash (see above) |
| encoding | `UTF-8` (excluded from hash) |
| oid | order ID |
| refreshTime | `5` |

---

## Response Parameters (returned by NestPay after 3D)

Key fields to parse:

| Field | Meaning |
|---|---|
| `Response` | `Approved` or `Decline` or `Error` |
| `mdStatus` | 3D status: `1`=full secure, `2`/`3`/`4`=half secure, others=failed |
| `AuthCode` | authorization code |
| `ReturnOid` | order ID |
| `ProcReturnCode` | `00` = success |
| `ErrMsg` | error message if failed |
| `HASH` | response hash for verification |
| `clientIp` | IP |
| `maskedCreditCard` | masked PAN |

### mdStatus handling:
- `1` → full 3D secure → proceed
- `2`, `3`, `4` → half secure → proceed with caution (merchant decision)
- anything else → authentication failed → decline

---

## Server-to-Server API (XML)

For non-3D or post-auth operations, POST XML to API URL with Basic Auth (api_username:api_password).

### Sale/Auth XML structure:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<CC5Request>
  <Name>gobibuyapi</Name>
  <Password>TEmp.291910@</Password>
  <ClientId>180000100</ClientId>
  <Type>Auth</Type>
  <OrderId>ORDER123</OrderId>
  <GroupId></GroupId>
  <TransId></TransId>
  <UserId></UserId>
  <Total>10.00</Total>
  <Currency>807</Currency>
  <Number>4799150896081734</Number>
  <Expires>12/28</Expires>
  <Cvv2Val>000</Cvv2Val>
  <Mode>P</Mode>
  <BillTo>
    <Name>John Doe</Name>
  </BillTo>
</CC5Request>
```

### Response XML structure:
```xml
<CC5Response>
  <OrderId>ORDER123</OrderId>
  <GroupId></GroupId>
  <Response>Approved</Response>
  <AuthCode>123456</AuthCode>
  <HostRefNum></HostRefNum>
  <ProcReturnCode>00</ProcReturnCode>
  <TransId></TransId>
  <ErrMsg></ErrMsg>
</CC5Response>
```

---

## Implement these payment flows

1. **PaymentAuthorize** — 3D form post (redirect flow using `RedirectForm`)
2. **PaymentSync** — poll transaction status via API XML
3. **Refund** — via API XML with `Type=Credit`
4. **RefundSync** — poll refund status

---

## ConnectorCommon implementation

```rust
fn id(&self) -> &'static str { "nestpay" }

fn get_currency_unit(&self) -> api::CurrencyUnit {
    api::CurrencyUnit::Base  // decimal amounts
}

fn get_auth_header(...) // NestPay uses form post + hash, not bearer token
// Return empty vec or not applicable

fn base_url(&self, connectors: &Connectors) -> &str {
    // 3D gate for redirect, API url for server-to-server
    connectors.nestpay.base_url.as_ref()
}
```

---

## Important implementation notes

1. For the 3D authorize flow, return a `RedirectForm::Form` with the 3D gate URL and all params as hidden fields including `encoding=UTF-8`
2. The hash must be computed AFTER all other parameters are finalized
3. Use `rand::distributions::DistString` for the `rnd` nonce (see globalpay transformers.rs for reference)
4. Never include `storeKey` as a visible form field — only use it for hash computation
5. Always verify the response `HASH` before treating a payment as successful
6. `okurl` and `failurl` must not redirect — they must return HTTP 200 directly (PCI DSS requirement)
7. Add `nestpay` to the connectors config in `config/development.toml` and `config/docker_compose.toml`

---

## Connector registration checklist

After creating the files, search the codebase for every place `globalpay` appears as a connector registration and add `nestpay` in the same pattern. Key places:
- `crates/hyperswitch_connectors/src/connectors.rs` → `pub mod nestpay;`
- Any `ConnectorEnum` or match statements listing all connectors
- `add_connector.md` mentions running a script — check if `scripts/add_connector.sh` exists and if so, note it but do NOT run it, implement manually
- `config/development.toml` — add `[connectors.nestpay] base_url = "https://torus-stage-halkbankmacedonia.asseco-see.com.tr"`

---

## Rust implementation guidance

- Use `serde` with `#[serde(rename = "clientid")]` etc for NestPay's mixed-case field names
- Use `hyperswitch_masking::Secret<String>` for store_key, api_password
- Use `common_utils::crypto::HmacSha512` or `Sha512` for hashing — check what's available in `common_utils::crypto`
- XML serialization: use `quick_xml` or `roxmltree` — check what's already in Cargo.toml before adding deps
- For form post: return `RedirectForm::Form { endpoint, params }` from the authorize transformer
- Error mapping: map `ProcReturnCode` != `"00"` to `ErrorResponse`
- Map `Response == "Approved"` to `AttemptStatus::Charged` or `Authorized` depending on capture mode