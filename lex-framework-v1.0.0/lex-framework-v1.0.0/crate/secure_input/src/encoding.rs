use ammonia::Builder as AmmoniaBuilder;
use quick_xml::se::{SeError as XmlError, to_string as xml_escape};
use regex::Regex;
use serde_json::to_string as json_escape;
use std::{borrow::Cow, fmt, sync::LazyLock};
use url::form_urlencoded;

fn html_builder() -> AmmoniaBuilder<'static> {
    let allowed_tags = ["a", "b", "em", "strong", "i", "span", "img"]
        .into_iter()
        .collect();
    let allowed_attrs = ["href", "title", "class", "alt"].into_iter().collect();
    let drop_content_tags = ["script", "iframe", "style", "noscript"]
        .into_iter()
        .collect();

    let mut builder = AmmoniaBuilder::default();
    builder
        .tags(allowed_tags)
        .generic_attributes(allowed_attrs)
        .url_relative(ammonia::UrlRelative::PassThrough)
        .clean_content_tags(drop_content_tags);
    builder
}

/// Clean HTML with the crate's conservative allow-list policy.
pub fn sanitize_html_whitelist(input: &str) -> String {
    html_builder().clean(input).to_string()
}

/// Serialize a string as a complete JSON string literal.
pub fn escape_json_string(input: &str) -> Result<String, serde_json::Error> {
    json_escape(input).map(|encoded| {
        encoded
            .replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029")
    })
}

#[derive(Debug)]
pub struct XmlEscapeError {
    pub message: String,
    pub source: XmlError,
}

impl fmt::Display for XmlEscapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "XML escape failed: {}", self.message)
    }
}

impl std::error::Error for XmlEscapeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Encode string values as XML text; serialize structured values as XML documents.
/// XML 1.0 does not permit arbitrary control characters, even when escaped.
pub fn escape_xml_text<T: serde::Serialize>(value: &T) -> Result<String, XmlEscapeError> {
    let serialized = serde_json::to_value(value).map_err(|error| XmlEscapeError {
        message: "could not serialize value".to_owned(),
        source: XmlError::Custom(error.to_string()),
    })?;
    if let serde_json::Value::String(text) = serialized {
        if text.chars().any(|character| !matches!(character as u32, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)) {
            return Err(XmlEscapeError { message: "invalid XML 1.0 text character".to_owned(), source: XmlError::Custom("invalid XML 1.0 text character".to_owned()) });
        }
        return Ok(quick_xml::escape::escape(text).into_owned());
    }
    xml_escape(value).map_err(|source| XmlEscapeError {
        message: "could not serialize value as XML".to_owned(),
        source,
    })
}

/// Escape a value for a JavaScript single-quoted string literal.
pub fn escape_js_str(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '<' => output.push_str("\\u003C"),
            '>' => output.push_str("\\u003E"),
            '&' => output.push_str("\\u0026"),
            '\\' => output.push_str("\\\\"),
            '\'' => output.push_str("\\'"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{2028}' => output.push_str("\\u2028"),
            '\u{2029}' => output.push_str("\\u2029"),
            character if (character as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(output, "\\u{:04X}", character as u32);
            }
            _ => output.push(character),
        }
    }
    output
}

/// Build a JavaScript assignment without treating `var` as executable source code.
pub fn js_assignment(var: &str, val: &str) -> String {
    format!(
        "globalThis['{}'] = '{}';",
        escape_js_str(var),
        escape_js_str(val)
    )
}

/// Escape a value for a CSS double-quoted string literal.
pub fn escape_css_string(input: &str) -> String {
    let mut output = String::with_capacity(input.len() + 2);
    output.push('"');
    for character in input.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\A "),
            '\r' => output.push_str("\\D "),
            character if (character as u32) < 0x20 || character == '\u{7F}' => {
                use std::fmt::Write as _;
                let _ = write!(output, "\\{:X} ", character as u32);
            }
            _ => output.push(character),
        }
    }
    output.push('"');
    output
}

pub fn css_content_rule(val: &str) -> String {
    format!("content: {};", escape_css_string(val))
}

pub fn encode_url_param(input: &str) -> Cow<'_, str> {
    form_urlencoded::byte_serialize(input.as_bytes()).collect()
}

pub fn build_query_string(params: &[(&str, &str)]) -> String {
    form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params.iter().copied())
        .finish()
}

static XSS_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)<\s*script",
        r"(?i)on\w+\s*=",
        r"(?i)javascript:",
        r#"(?i)src\s*=\s*['"]\s*data:"#,
    ]
    .into_iter()
    .map(|pattern| Regex::new(pattern).expect("built-in XSS regex must compile"))
    .collect()
});

/// Defense-in-depth lexical XSS detector. Context-specific output encoding is still required.
pub struct XssFilter;

impl XssFilter {
    pub fn new() -> Self {
        Self
    }

    pub fn filter(&self, input: &str) -> Result<String, String> {
        if XSS_PATTERNS.iter().any(|pattern| pattern.is_match(input)) {
            return Err("Potential XSS pattern detected".to_owned());
        }
        Ok(input.to_owned())
    }
}

impl Default for XssFilter {
    fn default() -> Self {
        Self::new()
    }
}

/// Legacy deterministic lexical content validator retained for API compatibility.
/// This is not an AI/ML moderation model.
pub struct AiValidator {
    forbidden: Vec<&'static str>,
}

impl AiValidator {
    pub fn new() -> Self {
        Self {
            forbidden: vec!["hate", "attack", "kill"],
        }
    }

    pub fn validate(&self, input: &str) -> Result<(), String> {
        for token in input
            .split(|character: char| !character.is_alphanumeric())
            .filter(|token| !token.is_empty())
        {
            if let Some(keyword) = self
                .forbidden
                .iter()
                .find(|keyword| token.eq_ignore_ascii_case(keyword))
            {
                return Err(format!("Disallowed content: '{keyword}'"));
            }
        }
        Ok(())
    }
}

impl Default for AiValidator {
    fn default() -> Self {
        Self::new()
    }
}

pub struct SanitizationPipeline {
    html: AmmoniaBuilder<'static>,
    xss: XssFilter,
    ai: AiValidator,
}

impl SanitizationPipeline {
    pub fn new() -> Self {
        Self {
            html: html_builder(),
            xss: XssFilter::new(),
            ai: AiValidator::new(),
        }
    }

    pub fn sanitize(&self, raw: &str) -> Result<String, String> {
        let cleaned_html = self.html.clean(raw).to_string();
        self.xss.filter(&cleaned_html)?;
        self.ai.validate(&cleaned_html)?;
        Ok(cleaned_html)
    }
}

impl Default for SanitizationPipeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_whitelist_removes_executable_content() {
        let output = sanitize_html_whitelist(r#"<script>alert(1)</script><b>Bold</b>"#);
        assert_eq!(output, "<b>Bold</b>");
    }

    #[test]
    fn html_whitelist_preserves_allowed_anchor_attributes() {
        let output = sanitize_html_whitelist(
            r#"<iframe src="evil"></iframe><a href="https://example.com" class="link">OK</a>"#,
        );
        assert!(output.contains(r#"href="https://example.com""#));
        assert!(output.contains(r#"class="link""#));
        assert!(output.contains(">OK</a>"));
        assert!(!output.contains("iframe"));
    }

    #[test]
    fn json_escaping_handles_control_and_unicode_separators() {
        let output = escape_json_string("Line\n\u{2028}Quote\"").expect("JSON serialization");
        assert!(output.starts_with('"') && output.ends_with('"'));
        assert!(output.contains("\\n") && output.contains("\\u2028") && output.contains("\\\""));
    }

    #[test]
    fn xml_escaping_and_errors_are_propagated() {
        #[derive(serde::Serialize)]
        struct Payload {
            value: String,
        }
        let xml = escape_xml_text(&Payload {
            value: "X&<>'\"".to_owned(),
        })
        .expect("XML serialization");
        assert!(xml.contains("&amp;") && xml.contains("&lt;") && xml.contains("&gt;"));

        struct Failing;
        impl serde::Serialize for Failing {
            fn serialize<S>(&self, _: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(serde::ser::Error::custom("forced failure"))
            }
        }
        assert!(escape_xml_text(&Failing).is_err());
    }

    #[test]
    fn javascript_escaping_handles_property_breakout_and_line_separators() {
        let assignment = js_assignment("x']; alert(1); //", "\u{2028}\u{2029}");
        assert!(assignment.starts_with("globalThis['"));
        assert!(assignment.contains("x\\']"));
        assert!(assignment.contains("\\u2028\\u2029"));
        assert!(assignment.ends_with("';"));
    }

    #[test]
    fn css_and_url_encoders_are_context_specific() {
        let css = escape_css_string("Hi\n\r\u{0000}");
        assert!(css.contains("\\A ") && css.contains("\\D ") && css.contains("\\0 "));
        assert_eq!(css_content_rule("X"), r#"content: "X";"#);
        let query = build_query_string(&[("k", "v v"), ("x", "1&2")]);
        assert!(query.contains("k=v+v") && query.contains("x=1%262"));
    }

    #[test]
    fn lexical_xss_filter_blocks_known_active_patterns() {
        let filter = XssFilter::new();
        assert!(filter.filter("<script>bad</script>").is_err());
        assert!(filter.filter(r#"<div onclick="do()"></div>"#).is_err());
        assert!(filter.filter(r#"<a href="JaVaScRiPt:evil">x</a>"#).is_err());
        assert!(filter.filter(r#"<img src="data:foo">"#).is_err());
        assert_eq!(
            filter.filter("plain text").expect("clean text"),
            "plain text"
        );
    }

    #[test]
    fn legacy_content_validator_matches_whole_words() {
        let validator = AiValidator::new();
        assert!(validator.validate("I will kill you").is_err());
        assert!(validator.validate("skill development").is_ok());
    }

    #[test]
    fn full_pipeline_sanitizes_markup_and_applies_content_policy() {
        let pipeline = SanitizationPipeline::new();
        assert!(
            pipeline
                .sanitize(r#"<img src="x.png"><p>kill</p>"#)
                .is_err()
        );
        let clean = pipeline
            .sanitize(r#"<p class="note">Welcome!</p>"#)
            .expect("clean content");
        assert!(clean.contains("Welcome!"));
    }
}
