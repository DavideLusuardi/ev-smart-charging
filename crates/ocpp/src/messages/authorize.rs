//! `Authorize`: the station asks whether an id token may start charging.

use serde::{Deserialize, Serialize};

use crate::bounded::{BoundedString, BoundedVec};
use crate::types::{CustomData, IdToken, IdTokenInfo};

/// `HashAlgorithmEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HashAlgorithm {
    #[serde(rename = "SHA256")]
    Sha256,
    #[serde(rename = "SHA384")]
    Sha384,
    #[serde(rename = "SHA512")]
    Sha512,
}

/// `OCSPRequestDataType`: data to check a certificate's revocation status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcspRequestData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub hash_algorithm: HashAlgorithm,
    pub issuer_name_hash: BoundedString<128>,
    pub issuer_key_hash: BoundedString<128>,
    pub serial_number: BoundedString<40>,
    #[serde(rename = "responderURL")]
    pub responder_url: BoundedString<512>,
}

/// `AuthorizeRequest`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorizeRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub id_token: IdToken,
    /// The EV's X.509 contract certificate, PEM encoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate: Option<BoundedString<5500>>,
    #[serde(
        rename = "iso15118CertificateHashData",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub iso15118_certificate_hash_data: Option<BoundedVec<OcspRequestData, 1, 4>>,
}

/// `AuthorizeCertificateStatusEnumType`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AuthorizeCertificateStatus {
    Accepted,
    SignatureError,
    CertificateExpired,
    CertificateRevoked,
    NoCertificateAvailable,
    CertChainError,
    ContractCancelled,
}

/// `AuthorizeResponse`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorizeResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<CustomData>,
    pub id_token_info: IdTokenInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate_status: Option<AuthorizeCertificateStatus>,
}
