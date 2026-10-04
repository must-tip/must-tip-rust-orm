use crate::error::ValidationError;
use regex::Regex;
use std::sync::LazyLock;
use unicode_normalization::UnicodeNormalization;

/// ANSI color codes for terminal output
const COLOR_RED: &str = "\x1b[31m";
const COLOR_YELLOW: &str = "\x1b[33m";
const COLOR_GREEN: &str = "\x1b[32m";
const COLOR_RESET: &str = "\x1b[0m";

/// Precompiled regexes for common checks
static COMMON_PASSWORDS_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Top common passwords + optional suffixes
    Regex::new(r"(?i)^(password|123456|qwerty|letmein|welcome|admin|abc123)[0-9!@#\$%\^&\*]*$")
        .expect("built-in common-password regex must compile")
});
static EMAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Simplified RFC-5322-style email validation
    Regex::new(r"(?i)^[a-z0-9_.+-]+@[a-z0-9-]+\.[a-z0-9-.]+$")
        .expect("built-in email regex must compile")
});

/// Strength levels for passwords
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordStrength {
    VeryWeak,
    Weak,
    Moderate,
    Strong,
    VeryStrong,
}

/// Normalize a string to Unicode NFKC (guards against look-alikes)
fn normalize(input: &str) -> String {
    input.nfkc().collect()
}

fn is_single_repeated_character(input: &str) -> bool {
    let mut chars = input.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    chars.next().is_some_and(|second| second == first) && input.chars().all(|ch| ch == first)
}

/// Evaluate password strength by scoring various criteria:
///   + Length points  
///   + Character variety (upper, lower, digit, symbol)  
///   + Deductions for common patterns or repeats  
pub fn evaluate_password_strength(password: &str) -> PasswordStrength {
    let pwd = normalize(password);
    let len = pwd.chars().count();

    // Base score starts with length; explicitly i32 so we can saturate‐subtract later
    let mut score: i32 = match len {
        0..=4 => 0,
        5..=7 => 1,
        8..=11 => 2,
        12..=15 => 3,
        _ => 4,
    };

    // +1 each for character‐class variety
    if pwd.chars().any(|c| c.is_ascii_lowercase()) {
        score += 1;
    }
    if pwd.chars().any(|c| c.is_ascii_uppercase()) {
        score += 1;
    }
    if pwd.chars().any(|c| c.is_ascii_digit()) {
        score += 1;
    }
    if pwd.chars().any(|c| !c.is_alphanumeric()) {
        score += 1;
    }

    // -2 if too common
    if COMMON_PASSWORDS_RE.is_match(&pwd) {
        score = score.saturating_sub(2);
    }
    // -2 if single char repeated
    if is_single_repeated_character(&pwd) {
        score = score.saturating_sub(2);
    }

    // Clamp between 0 and 6
    let score = score.clamp(0, 6);

    // Map to strength level
    match score {
        0 | 1         => PasswordStrength::VeryWeak,
        2             => PasswordStrength::Weak,
        3 | 4         => PasswordStrength::Moderate,
        5             => PasswordStrength::Strong,
        _ /* 6 */     => PasswordStrength::VeryStrong,
    }
}

/// Return an ANSI-colored label for a strength level (for CLI)
pub fn colored_strength_label(strength: &PasswordStrength) -> String {
    let (label, color) = match strength {
        PasswordStrength::VeryWeak => ("Very Weak", COLOR_RED),
        PasswordStrength::Weak => ("Weak", COLOR_RED),
        PasswordStrength::Moderate => ("Moderate", COLOR_YELLOW),
        PasswordStrength::Strong => ("Strong", COLOR_GREEN),
        PasswordStrength::VeryStrong => ("Very Strong", COLOR_GREEN),
    };
    format!("{color}{label}{COLOR_RESET}")
}

/// Validate that a password meets minimum security policy:
///   - At least 8 chars  
///   - At least 3 of 4 character classes  
///   - Not a top-common password  
///   - Not a simple repeat  
pub fn validate_password(password: &str) -> Result<(), ValidationError> {
    let pwd = normalize(password);

    if pwd.chars().count() < 8 {
        return Err(ValidationError::InvalidPassword);
    }

    // Count character classes
    let classes = [
        pwd.chars().any(|c| c.is_ascii_lowercase()),
        pwd.chars().any(|c| c.is_ascii_uppercase()),
        pwd.chars().any(|c| c.is_ascii_digit()),
        pwd.chars().any(|c| !c.is_alphanumeric()),
    ]
    .iter()
    .filter(|&&b| b)
    .count();

    if classes < 3 {
        return Err(ValidationError::InvalidPassword);
    }
    if COMMON_PASSWORDS_RE.is_match(&pwd) || is_single_repeated_character(&pwd) {
        return Err(ValidationError::InvalidPassword);
    }

    Ok(())
}

/// Validate an email address using normalization and a simplified RFC pattern
pub fn validate_email(email: &str) -> Result<(), ValidationError> {
    let norm = normalize(email);

    if norm.len() > 320 || !EMAIL_RE.is_match(&norm) {
        return Err(ValidationError::InvalidEmail);
    }
    // single '@' and no consecutive dots
    if norm.matches('@').count() != 1 || norm.contains("..") {
        return Err(ValidationError::InvalidEmail);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_character_detection_is_safe_and_correct() {
        assert!(is_single_repeated_character("aaaaaaaa"));
        assert!(is_single_repeated_character("🔥🔥🔥"));
        assert!(!is_single_repeated_character("a"));
        assert!(!is_single_repeated_character("abababab"));
        assert!(!is_single_repeated_character(""));
    }

    #[test]
    fn repeated_character_password_is_rejected_without_regex_backreferences() {
        assert!(matches!(
            validate_password("AAAAAAAA"),
            Err(ValidationError::InvalidPassword)
        ));
    }
}
