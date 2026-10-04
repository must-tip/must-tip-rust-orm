use regex::Regex;
use std::sync::LazyLock;

/// --- Precompiled Regex Patterns ---
/// Detects SQL Injection attempts.
pub static SQLI_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?xi)
        (
            \b(SELECT|INSERT|DELETE|UPDATE|DROP|UNION|ALTER|CREATE)\b  # core SQL keywords
            |
            (--|;|\bOR\s+1\s*=\s*1\b|\bAND\s+1\s*=\s*1\b)               # common SQLi logic
            |
            (/\*.*?\*/|\#)                                              # comment tricks
            |
            \b0x[0-9A-Fa-f]{4,}\b                                      # hex literal injection
        )
    ",
    )
    .expect("built-in regular expression must compile")
});

/// Lexical heuristic for padded Base64 values and long unpadded sequences.
/// This is not a substitute for decoding or context-specific output encoding.
pub static BASE64_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^[A-Za-z0-9+/]{4}(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)$|[A-Za-z0-9+/]{64,}={0,2})").expect("built-in regular expression must compile")
});

/// Detects suspicious script-related file extensions.
pub static DANGEROUS_EXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(exe|bat|cmd|scr|ps1|vbs|js|wsf|msi|dll|sh|py)$")
        .expect("built-in regular expression must compile")
});

/// Detects signs of obfuscated JavaScript code.
pub static OBFUSCATED_JS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?xi)(eval\s*\(|unescape\s*\(|document\.write\s*\(|%[0-9A-F]{2})")
        .expect("built-in regular expression must compile")
});

/// Detects raw HTML tags in input.
pub static HTML_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<[^>]+>").expect("built-in regular expression must compile"));

/// Detects control characters, typically non-printable and potentially malicious.
pub static CONTROL_CHAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\x00-\x1F\x7F]").expect("built-in regular expression must compile")
});

/// Detects path traversal attempts.
pub static PATH_TRAVERSAL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\.\./|%2e%2e)").expect("built-in regular expression must compile")
});

/// Validates usernames: 3–30 chars, starts with letter/underscore, only alphanumeric or underscore.
pub static USERNAME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z_][A-Za-z0-9_]{2,29}$").expect("built-in regular expression must compile")
});

/// Validates a hex string (commonly used for token validation or encoding).
pub static HEX_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[0-9A-Fa-f]+$").expect("built-in regular expression must compile")
});

/// Validates phone numbers in E.164 format (e.g. +1234567890).
pub static PHONE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\+?[1-9]\d{7,14}$").expect("built-in regular expression must compile")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_security_patterns_compile_and_initialize() {
        assert!(SQLI_RE.is_match("DROP TABLE users;"));
        assert!(SQLI_RE.is_match("#"));
        assert!(BASE64_RE.is_match(&"A".repeat(64)));
        assert!(DANGEROUS_EXT_RE.is_match("exe"));
        assert!(OBFUSCATED_JS_RE.is_match("eval("));
        assert!(HTML_TAG_RE.is_match("<b>"));
        assert!(CONTROL_CHAR_RE.is_match("\u{0000}"));
        assert!(PATH_TRAVERSAL_RE.is_match("../secret"));
        assert!(USERNAME_RE.is_match("user_123"));
        assert!(HEX_RE.is_match("deadBEEF"));
        assert!(PHONE_RE.is_match("+12345678901"));
    }
}
