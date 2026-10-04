// src/validation.rs

use crate::error::ValidationError;
use crate::types::{
    BASE64_RE, CONTROL_CHAR_RE, DANGEROUS_EXT_RE, HEX_RE, HTML_TAG_RE, OBFUSCATED_JS_RE, PHONE_RE,
    SQLI_RE, USERNAME_RE,
};
use percent_encoding::percent_decode_str;
use std::{net::IpAddr, path::Path, str::FromStr};
use unicode_normalization::UnicodeNormalization;
use url::Url;
use uuid::Uuid;

/// Validators take an &str and either succeed or return a ValidationError.
pub trait Validator {
    fn validate(&self, input: &str) -> Result<(), ValidationError>;
}

/// Allow bare functions `fn(&str)->Result<(),ValidationError>` to be Validators.
impl Validator for fn(&str) -> Result<(), ValidationError> {
    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        (self)(input)
    }
}

/// Normalize to Unicode NFKC once per call to defend against homoglyphs.
fn normalize_nfkc(input: &str) -> Result<String, ValidationError> {
    Ok(input.nfkc().collect())
}

/// Generic helper: run a Regex, map first match to a custom error.
fn regex_validate(
    input: &str,
    re: &regex::Regex,
    make_err: impl Fn(&str) -> ValidationError,
) -> Result<(), ValidationError> {
    if let Some(m) = re.find(input) {
        Err(make_err(m.as_str()))
    } else {
        Ok(())
    }
}

/// ——— Validators using `normalize_nfkc` + regex checks ———
/// Username: 3–30 chars, start with letter/underscore, then alphanumeric/underscore.
pub fn validate_username(input: &str) -> Result<(), ValidationError> {
    let s = normalize_nfkc(input)?;
    if USERNAME_RE.is_match(&s) {
        Ok(())
    } else {
        Err(ValidationError::InvalidUsername)
    }
}

/// URL parsing via `url` crate.
pub fn validate_url(input: &str) -> Result<(), ValidationError> {
    let normalized = normalize_nfkc(input)?;
    let url = Url::parse(&normalized).map_err(|_| ValidationError::InvalidURL)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ValidationError::InvalidURL);
    }
    Ok(())
}

/// UUID parsing via `uuid` crate.
pub fn validate_uuid(input: &str) -> Result<(), ValidationError> {
    Uuid::parse_str(input)
        .map_err(|_| ValidationError::InvalidUUID)
        .map(|_| ())
}

/// IP address parsing via `std::net::IpAddr`.
pub fn validate_ip(input: &str) -> Result<(), ValidationError> {
    IpAddr::from_str(input)
        .map_err(|_| ValidationError::InvalidIP)
        .map(|_| ())
}

/// Hex string check.
pub fn validate_hex(input: &str) -> Result<(), ValidationError> {
    let s = normalize_nfkc(input)?;
    if HEX_RE.is_match(&s) {
        Ok(())
    } else {
        Err(ValidationError::InvalidHex)
    }
}

/// E.164 phone format check.
pub fn validate_phone(input: &str) -> Result<(), ValidationError> {
    let s = normalize_nfkc(input)?;
    if PHONE_RE.is_match(&s) {
        Ok(())
    } else {
        Err(ValidationError::InvalidPhone)
    }
}

/// ——— “Payload” validator: SQLi, Base64, JS obfuscation, HTML tags, control chars ———
pub fn validate_input(input: &str) -> Result<(), ValidationError> {
    let s = normalize_nfkc(input)?;

    // each tuple is (Regex, fn(matched) -> ValidationError)
    type PayloadCheck<'a> = (&'a regex::Regex, &'a dyn Fn(&str) -> ValidationError);
    let checks: &[PayloadCheck<'_>] = &[
        (&SQLI_RE, &|m| ValidationError::SqlInjection(m.to_string())),
        (&BASE64_RE, &|_| ValidationError::Base64Payload),
        (&OBFUSCATED_JS_RE, &|m| {
            ValidationError::ObfuscatedJs(m.to_string())
        }),
        (&HTML_TAG_RE, &|_| {
            ValidationError::ObfuscatedJs("<html_tag>".into())
        }),
        (&CONTROL_CHAR_RE, &|_| ValidationError::NormalizationError),
    ];

    for (re, mk_err) in checks {
        regex_validate(&s, re, *mk_err)?;
    }

    Ok(())
}

/// ——— Filename safety: decode, normalize, reject traversal or bad ext ———
pub fn validate_filename(raw: &str) -> Result<(), ValidationError> {
    let decoded = percent_decode_str(raw)
        .decode_utf8()
        .map_err(|_| ValidationError::UnsafeFilename)?;
    let normalized: String = decoded.nfkc().collect();
    if normalized.is_empty()
        || normalized.len() > 255
        || normalized.chars().any(|ch| matches!(ch, '\0' | '/' | '\\'))
    {
        return Err(ValidationError::UnsafeFilename);
    }
    let path = Path::new(&normalized);
    let mut components = path.components();
    if !matches!(components.next(), Some(std::path::Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(ValidationError::UnsafeFilename);
    }
    if let Some(extension) = path.extension().and_then(|value| value.to_str())
        && DANGEROUS_EXT_RE.is_match(extension)
    {
        return Err(ValidationError::UnsafeFilename);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_validator_rejects_active_and_local_schemes() {
        assert!(validate_url("https://example.com/path").is_ok());
        assert!(validate_url("javascript:alert(1)").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn filename_validator_rejects_traversal_and_encoded_traversal() {
        assert!(validate_filename("report.pdf").is_ok());
        assert!(validate_filename("../report.pdf").is_err());
        assert!(validate_filename("%2e%2e%2freport.pdf").is_err());
        assert!(validate_filename("nested/report.pdf").is_err());
        assert!(validate_filename("payload.exe").is_err());
    }
}
