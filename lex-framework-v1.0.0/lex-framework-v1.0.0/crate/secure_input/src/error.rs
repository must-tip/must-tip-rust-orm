use crate::encoding::XmlEscapeError;
use thiserror::Error;

impl From<XmlEscapeError> for ValidationError {
    fn from(e: XmlEscapeError) -> Self {
        ValidationError::Custom(format!("XML escaping failed: {}", e))
    }
}

impl From<serde_json::Error> for ValidationError {
    fn from(e: serde_json::Error) -> Self {
        ValidationError::Custom(format!("JSON escaping failed: {}", e))
    }
}
/// Structured validation errors
#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Invalid email address")]
    InvalidEmail,

    #[error("Invalid username (3–30 chars, start with letter/_ , letters/digits/_ only)")]
    InvalidUsername,

    #[error("Weak password (min 8 chars, include uppercase, lowercase, digit, symbol)")]
    InvalidPassword,

    #[error("Invalid URL")]
    InvalidURL,

    #[error("Invalid UUID")]
    InvalidUUID,

    #[error("Invalid IP address")]
    InvalidIP,

    #[error("Invalid hex string")]
    InvalidHex,

    #[error("Invalid phone number (E.164 format)")]
    InvalidPhone,

    #[error("Potential SQL injection pattern detected: `{0}`")]
    SqlInjection(String),

    #[error("Detected large Base64 payload")]
    Base64Payload,

    #[error("Obfuscated JavaScript detected: `{0}`")]
    ObfuscatedJs(String),

    #[error("Filename not safe (path traversal or dangerous extension)")]
    UnsafeFilename,

    #[error("Input normalization failed")]
    NormalizationError,

    #[error("Custom validation error: {0}")]
    Custom(String),
}
