use crate::error::ValidationError;
use unicode_normalization::UnicodeNormalization;

/// A processing stage that can mutate or reject an input string.
pub trait Filter {
    fn filter(&self, input: &str) -> Result<String, ValidationError>;
}

impl Filter for fn(&str) -> Result<String, ValidationError> {
    fn filter(&self, input: &str) -> Result<String, ValidationError> {
        (self)(input)
    }
}

/// Normalize Unicode using NFKC.
pub fn normalize(input: &str) -> Result<String, ValidationError> {
    Ok(input.nfkc().collect())
}

/// Run the canonical payload validator.
pub fn validate_input(input: &str) -> Result<(), ValidationError> {
    crate::validate::validate_input(input)
}

/// Run the canonical filename validator.
pub fn validate_filename(filename: &str) -> Result<(), ValidationError> {
    crate::validate::validate_filename(filename)
}

/// Pure predicate for callers that only need a boolean decision.
pub fn is_clean_payload(input: &str) -> bool {
    crate::validate::validate_input(input).is_ok()
}
