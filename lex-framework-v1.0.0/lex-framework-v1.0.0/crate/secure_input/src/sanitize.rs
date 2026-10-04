use crate::{
    error::ValidationError,
    types::{BASE64_RE, DANGEROUS_EXT_RE, PATH_TRAVERSAL_RE, SQLI_RE},
};
use htmlescape::encode_minimal;
use regex::Regex;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

/// Sanitizer trait: checks/cleans strings, must be thread-safe.
pub trait Sanitizer: Send + Sync {
    fn sanitize(&self, input: &str) -> String;
}

/// Adapter: wrap bare function pointer so it can be used as a Sanitizer.
pub struct FnSanitizer(pub fn(&str) -> String);

impl Sanitizer for FnSanitizer {
    fn sanitize(&self, input: &str) -> String {
        (self.0)(input)
    }
}

// Regex patterns for detection & cleaning
static RE_NON_ASCII: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[^\x00-\x7F]").expect("built-in regular expression must compile")
});
static RE_CONTROL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\x00-\x1F\x7F]").expect("built-in regular expression must compile")
});
static RE_WHITESPACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+").expect("built-in regular expression must compile"));
static RE_SPECIAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\w\s]").expect("built-in regular expression must compile"));
static RE_HTML_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<[^>]+>").expect("built-in regular expression must compile"));
static RE_DOUBLE_ENCODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)%25|%2[eE]%2[eE]").expect("built-in regular expression must compile")
});
static RE_SCRIPT_KW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(script|alert|onerror|onload|javascript:)\b")
        .expect("built-in regular expression must compile")
});
static RE_EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}$")
        .expect("built-in regular expression must compile")
});

// --- Sanitizer functions --- //

pub fn normalize_unicode(input: &str) -> String {
    input.nfkc().collect()
}

pub fn sanitize_string(input: &str) -> String {
    let s = input.trim();
    let s = RE_NON_ASCII.replace_all(s, "");
    RE_CONTROL.replace_all(&s, "").to_string()
}

pub fn sanitize_html(input: &str) -> String {
    RE_HTML_TAG.replace_all(input, "").to_string()
}

pub fn collapse_whitespace(input: &str) -> String {
    RE_WHITESPACE.replace_all(input.trim(), " ").to_string()
}

pub fn remove_special_chars(input: &str) -> String {
    RE_SPECIAL.replace_all(input, "").to_string()
}

pub fn sanitize_numeric(input: &str) -> String {
    input.chars().filter(|c| c.is_ascii_digit()).collect()
}

pub fn escape_html(input: &str) -> String {
    encode_minimal(input)
}

/// Chain multiple sanitizers safely using `Arc<dyn Sanitizer>`
pub fn chain(input: &str, sanitizers: &[&dyn Sanitizer]) -> String {
    sanitizers
        .iter()
        .fold(input.to_string(), |acc, s| s.sanitize(&acc))
}

// --- Validation guards --- //

pub fn reject_sqli(input: &str) -> Result<&str, ValidationError> {
    if SQLI_RE.is_match(input) {
        Err(ValidationError::Custom("SQL injection detected".into()))
    } else {
        Ok(input)
    }
}

pub fn reject_base64(input: &str) -> Result<&str, ValidationError> {
    if BASE64_RE.is_match(input) {
        Err(ValidationError::Custom("Encoded payload detected".into()))
    } else {
        Ok(input)
    }
}

pub fn reject_script(input: &str) -> Result<&str, ValidationError> {
    if RE_SCRIPT_KW.is_match(input) {
        Err(ValidationError::Custom(
            "Potential XSS keyword detected".into(),
        ))
    } else {
        Ok(input)
    }
}

pub fn reject_dangerous_ext(name: &str) -> Result<&str, ValidationError> {
    let lower = name.to_lowercase();
    if DANGEROUS_EXT_RE.is_match(lower.rsplit_once('.').map_or("", |(_, ext)| ext))
        || RE_DOUBLE_ENCODE.is_match(&lower)
        || PATH_TRAVERSAL_RE.is_match(name)
    {
        Err(ValidationError::Custom("Unsafe filename".into()))
    } else {
        Ok(name)
    }
}

pub fn validate_email(input: &str) -> bool {
    RE_EMAIL.is_match(input)
}

pub fn enforce_length(input: &str, max: usize) -> Result<&str, ValidationError> {
    if input.chars().count() > max {
        Err(ValidationError::Custom("Input too long".into()))
    } else {
        Ok(input)
    }
}
