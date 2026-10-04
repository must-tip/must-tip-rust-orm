// src/pipeline.rs

use crate::encoding;
use crate::error::ValidationError;
use crate::filter::Filter;
use crate::sanitize::Sanitizer;
use crate::sanitize::{
    chain, collapse_whitespace, enforce_length, escape_html, normalize_unicode, reject_base64,
    reject_dangerous_ext, reject_script, reject_sqli, remove_special_chars, sanitize_html,
    sanitize_numeric, sanitize_string, validate_email,
};
use crate::validate::Validator;
use crate::validate::{
    validate_filename, validate_hex, validate_input, validate_ip, validate_phone, validate_url,
    validate_username, validate_uuid,
};

/// Core Stage trait: consumes a String, returns String or ValidationError.
pub trait Stage {
    fn name(&self) -> &'static str;
    fn apply(&self, input: String) -> Result<String, ValidationError>;
}

/// Wrap any Sanitizer + name.
pub struct SanitizerStage<S> {
    pub name: &'static str,
    pub sanitizer: S,
}
impl<S> Stage for SanitizerStage<S>
where
    S: Sanitizer + Send + Sync + 'static,
{
    fn name(&self) -> &'static str {
        self.name
    }
    fn apply(&self, input: String) -> Result<String, ValidationError> {
        Ok(self.sanitizer.sanitize(&input))
    }
}

/// Wrap any Validator + name.
pub struct ValidatorStage<V> {
    pub name: &'static str,
    pub validator: V,
}
impl<V> Stage for ValidatorStage<V>
where
    V: Validator + Send + Sync + 'static,
{
    fn name(&self) -> &'static str {
        self.name
    }
    fn apply(&self, input: String) -> Result<String, ValidationError> {
        self.validator
            .validate(&input)
            .map_err(|e| ValidationError::Custom(format!("[{}] {}", self.name, e)))?;
        Ok(input)
    }
}

/// Wrap any Filter + name.
pub struct FilterStage<F> {
    pub name: &'static str,
    pub filter: F,
}
impl<F> Stage for FilterStage<F>
where
    F: Filter + Send + Sync + 'static,
{
    fn name(&self) -> &'static str {
        self.name
    }
    fn apply(&self, input: String) -> Result<String, ValidationError> {
        self.filter
            .filter(&input)
            .map_err(|e| ValidationError::Custom(format!("[{}] {}", self.name, e)))
    }
}

/// Adapter: fn(&str)->String → Sanitizer
pub struct FnSanitizer<F>(pub F);

impl<F> Sanitizer for FnSanitizer<F>
where
    F: Fn(&str) -> String + Send + Sync + 'static,
{
    fn sanitize(&self, input: &str) -> String {
        (self.0)(input)
    }
}

/// Adapter: fn(&str)->Result<String,E> → Filter
struct FnFilter<F>(F);
impl<F, E> Filter for FnFilter<F>
where
    F: Fn(&str) -> Result<String, E> + Send + Sync + 'static,
    ValidationError: From<E>,
{
    fn filter(&self, input: &str) -> Result<String, ValidationError> {
        (self.0)(input).map_err(ValidationError::from)
    }
}

/// Adapter: fn(&str)->Result<(),E> → Validator
struct FnValidator<F>(F);
impl<F, E> Validator for FnValidator<F>
where
    F: Fn(&str) -> Result<(), E> + Send + Sync + 'static,
    ValidationError: From<E>,
{
    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        (self.0)(input).map_err(ValidationError::from)
    }
}

/// The pipeline builder.
pub struct Pipeline {
    stages: Vec<Box<dyn Stage + Send + Sync>>,
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl Pipeline {
    pub fn new() -> Self {
        Pipeline { stages: Vec::new() }
    }

    pub fn add_stage<S>(mut self, stage: S) -> Self
    where
        S: Stage + Send + Sync + 'static,
    {
        self.stages.push(Box::new(stage));
        self
    }

    pub fn sanitizer<S>(self, name: &'static str, s: S) -> Self
    where
        S: Sanitizer + Send + Sync + 'static,
    {
        self.add_stage(SanitizerStage { name, sanitizer: s })
    }

    pub fn filter<F>(self, name: &'static str, f: F) -> Self
    where
        F: Filter + Send + Sync + 'static,
    {
        self.add_stage(FilterStage { name, filter: f })
    }

    pub fn validator<V>(self, name: &'static str, v: V) -> Self
    where
        V: Validator + Send + Sync + 'static,
    {
        self.add_stage(ValidatorStage { name, validator: v })
    }

    pub fn sanitizer_fn<F>(self, name: &'static str, f: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.sanitizer(name, FnSanitizer(f))
    }

    pub fn filter_fn<F, E>(self, name: &'static str, f: F) -> Self
    where
        F: Fn(&str) -> Result<String, E> + Send + Sync + 'static,
        ValidationError: From<E>,
    {
        self.filter(name, FnFilter(f))
    }

    pub fn validator_fn<F, E>(self, name: &'static str, f: F) -> Self
    where
        F: Fn(&str) -> Result<(), E> + Send + Sync + 'static,
        ValidationError: From<E>,
    {
        self.validator(name, FnValidator(f))
    }

    // ── Built-ins from encoding.rs ───────────────────────────────

    pub fn html_whitelist(self) -> Self {
        self.sanitizer_fn("html_whitelist", encoding::sanitize_html_whitelist)
    }
    pub fn escape_json(self) -> Self {
        self.filter_fn("escape_json", |s| {
            encoding::escape_json_string(s).map_err(ValidationError::from)
        })
    }

    pub fn escape_xml(self) -> Self {
        self.filter_fn("escape_xml", |s| {
            encoding::escape_xml_text(&s).map_err(ValidationError::from)
        })
    }

    pub fn escape_js(self) -> Self {
        self.sanitizer_fn("escape_js", encoding::escape_js_str)
    }
    pub fn js_assign(self, var: &'static str) -> Self {
        self.sanitizer_fn("js_assign", move |s| encoding::js_assignment(var, s))
    }
    pub fn escape_css(self) -> Self {
        self.sanitizer_fn("escape_css", encoding::escape_css_string)
    }
    pub fn css_content(self) -> Self {
        self.sanitizer_fn("css_content", encoding::css_content_rule)
    }
    pub fn encode_url(self) -> Self {
        self.sanitizer_fn("encode_url", |s| encoding::encode_url_param(s).into_owned())
    }
    pub fn build_query(self, params: &'static [(&'static str, &'static str)]) -> Self {
        self.sanitizer_fn("build_query", move |_| encoding::build_query_string(params))
    }

    // ── From validation.rs ────────────────────────────────────────

    pub fn validate_username(self) -> Self {
        self.validator_fn("validate_username", validate_username)
    }
    pub fn validate_url(self) -> Self {
        self.validator_fn("validate_url", validate_url)
    }
    pub fn validate_uuid(self) -> Self {
        self.validator_fn("validate_uuid", validate_uuid)
    }
    pub fn validate_ip(self) -> Self {
        self.validator_fn("validate_ip", validate_ip)
    }
    pub fn validate_hex(self) -> Self {
        self.validator_fn("validate_hex", validate_hex)
    }
    pub fn validate_phone(self) -> Self {
        self.validator_fn("validate_phone", validate_phone)
    }
    pub fn payload_check(self) -> Self {
        self.validator_fn("payload_check", validate_input)
    }
    pub fn validate_filename(self) -> Self {
        self.validator_fn("validate_filename", validate_filename)
    }

    // ── From sanitizer.rs ─────────────────────────────────────────

    pub fn normalize_unicode(self) -> Self {
        self.sanitizer_fn("normalize_unicode", normalize_unicode)
    }
    pub fn sanitize_string(self) -> Self {
        self.sanitizer_fn("sanitize_string", sanitize_string)
    }
    pub fn sanitize_html(self) -> Self {
        self.sanitizer_fn("sanitize_html", sanitize_html)
    }
    pub fn collapse_whitespace(self) -> Self {
        self.sanitizer_fn("collapse_whitespace", collapse_whitespace)
    }
    pub fn remove_special_chars(self) -> Self {
        self.sanitizer_fn("remove_special_chars", remove_special_chars)
    }
    pub fn sanitize_numeric(self) -> Self {
        self.sanitizer_fn("sanitize_numeric", sanitize_numeric)
    }
    pub fn escape_html(self) -> Self {
        self.sanitizer_fn("escape_html", escape_html)
    }

    pub fn chain(self, sanitisers: &'static [&'static dyn Sanitizer]) -> Self {
        self.sanitizer_fn("chain", move |s| chain(s, sanitisers))
    }

    pub fn reject_sqli(self) -> Self {
        self.filter_fn("reject_sqli", |s| reject_sqli(s).map(|o| o.to_string()))
    }
    pub fn reject_base64(self) -> Self {
        self.filter_fn("reject_base64", |s| reject_base64(s).map(|o| o.to_string()))
    }
    pub fn reject_script(self) -> Self {
        self.filter_fn("reject_script", |s| reject_script(s).map(|o| o.to_string()))
    }
    pub fn reject_dangerous_ext(self) -> Self {
        self.filter_fn("reject_dangerous_ext", |s| {
            reject_dangerous_ext(s).map(|o| o.to_string())
        })
    }
    pub fn validate_email(self) -> Self {
        self.filter_fn("validate_email", |s| {
            if validate_email(s) {
                Ok(s.to_string())
            } else {
                Err(ValidationError::Custom("Invalid email".into()))
            }
        })
    }
    pub fn enforce_length(self, max: usize) -> Self {
        self.filter_fn("enforce_length", move |s| {
            enforce_length(s, max).map(|o| o.to_string())
        })
    }

    /// Run through all stages in order.
    pub fn run(&self, input: &str) -> Result<String, ValidationError> {
        let mut data = input.to_string();
        for stage in &self.stages {
            data = stage.apply(data)?;
        }
        Ok(data)
    }
}

/// Inline macro for ad-hoc pipelines.
#[macro_export]
macro_rules! pipeline {
    ( $($stage:expr),* $(,)? ) => {{
        let mut p = $crate::pipeline::Pipeline::new();
        $( p = p.add_stage($stage); )*
        p
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ValidationError;

    // Helper: run a single‐stage pipeline
    fn run_stage(p: Pipeline, input: &str) -> Result<String, ValidationError> {
        p.run(input)
    }

    #[test]
    fn test_html_whitelist() {
        let raw = r#"<b>Hi</b><script>alert(1)</script>"#;
        let p = Pipeline::new().html_whitelist();
        let out = run_stage(p, raw).unwrap();
        assert_eq!(out, "<b>Hi</b>");
    }

    #[test]
    fn test_escape_json_and_xml() {
        let s = "Line\n\"Foo\"";
        let p1 = Pipeline::new().escape_json();
        assert_eq!(run_stage(p1, s).unwrap(), "\"Line\\n\\\"Foo\\\"\"");
        let p2 = Pipeline::new().escape_xml();
        let xml = run_stage(p2, s).unwrap();
        // must contain &quot; and &#10; or &amp;amp; &lt; etc.
        assert!(xml.contains("&quot;") && xml.contains("Line"));
    }

    #[test]
    fn test_js_and_css_and_url() {
        let s = "O'Reilly\nTest";
        let p = Pipeline::new()
            .escape_js()
            .js_assign("v")
            .escape_css()
            .css_content()
            .encode_url();
        let out = run_stage(p, s).unwrap();
        let decoded: String = url::form_urlencoded::parse(format!("value={out}").as_bytes())
            .next()
            .unwrap()
            .1
            .into_owned();
        assert!(decoded.starts_with("content: \""));
        assert!(decoded.contains("globalThis["));
        assert!(!decoded.contains('\n'));
    }

    #[test]
    fn test_build_query() {
        let p = Pipeline::new().build_query(&[("a", "1"), ("b", "x y")]);
        let out = run_stage(p, "").unwrap();
        assert!(out.contains("a=1") && out.contains("b=x+y"));
    }

    #[test]
    fn test_validate_username_url_uuid_ip_hex_phone() {
        // username ok / err
        let ok = Pipeline::new().validate_username();
        assert!(ok.run("user_123").is_ok());
        let bad = Pipeline::new().validate_username();
        assert!(bad.run("1bad").is_err());

        // URL
        let ok = Pipeline::new().validate_url();
        assert!(ok.run("https://rust-lang.org").is_ok());
        assert!(ok.run("not_a_url").is_err());

        // UUID
        let ok = Pipeline::new().validate_uuid();
        assert!(ok.run("550e8400-e29b-41d4-a716-446655440000").is_ok());
        assert!(ok.run("invalid-uuid").is_err());

        // IP
        let ok = Pipeline::new().validate_ip();
        assert!(ok.run("127.0.0.1").is_ok());
        assert!(ok.run("999.999.999.999").is_err());

        // Hex
        let ok = Pipeline::new().validate_hex();
        assert!(ok.run("deadBEEF").is_ok());
        assert!(ok.run("xyz").is_err());

        // Phone
        let ok = Pipeline::new().validate_phone();
        assert!(ok.run("+1234567890").is_ok());
        assert!(ok.run("123-abc").is_err());
    }

    #[test]
    fn test_payload_and_filename_checks() {
        // payload_check blocks SQLi
        let p = Pipeline::new().payload_check();
        assert!(p.run("DROP TABLE users;").is_err());
        assert!(p.run("normal text").is_ok());

        // validate_filename ok / err
        let p = Pipeline::new().validate_filename();
        assert!(p.run("safe.txt").is_ok());
        assert!(p.run("../evil.sh").is_err());
    }

    #[test]
    fn test_sanitizer_helpers() {
        let raw = "Áß 123\t\n<script>";
        let p = Pipeline::new()
            .normalize_unicode()
            .sanitize_string()
            .sanitize_html()
            .sanitize_numeric();
        let out = run_stage(p, raw).unwrap();
        // Numeric sanitization keeps digits regardless of preceding normalization.
        assert_eq!(out, "123");
    }

    #[test]
    fn test_whitespace_special_email_length() {
        let raw = "   hello   \nworld!!!   ";
        let p = Pipeline::new()
            .collapse_whitespace()
            .remove_special_chars()
            .validate_email() // will Err on "helloworld"
            .enforce_length(20);
        let res = p.run(raw);
        assert!(matches!(res, Err(ValidationError::Custom(_))));
    }

    #[test]
    fn test_reject_filters() {
        let p1 = Pipeline::new().reject_sqli();
        assert!(p1.run("SELECT *").is_err());
        let p2 = Pipeline::new().reject_base64();
        assert!(p2.run("QUJDREVGR0g=").is_err());
        let p3 = Pipeline::new().reject_script();
        assert!(p3.run("onerror=1").is_err());
        let p4 = Pipeline::new().reject_dangerous_ext();
        assert!(p4.run("file.exe").is_err());
    }

    #[test]
    fn test_chain_macro_and_run_sequence() {
        use crate::ValidationError;
        use crate::pipeline::{FnSanitizer, SanitizerStage, ValidatorStage};
        use crate::sanitize::sanitize_html;
        use crate::validate::validate_username;

        // Wrap function pointers in types that implement the traits
        // let sanitizer = FnSanitizer(sanitize_html_whitelist);
        let validator = validate_username as fn(&str) -> Result<(), ValidationError>;

        // Construct dynamic stage list
        // let stages: &[&dyn Stage] = &[
        //     &SanitizerStage { name: "wh", sanitizer: sanitizer },
        //     &ValidatorStage { name: "un", validator: validator },
        // ];

        // Create the pipeline using macro (assumes `pipeline!` macro is correctly defined)
        let p = pipeline![
            SanitizerStage {
                name: "wh",
                sanitizer: FnSanitizer(sanitize_html)
            },
            ValidatorStage {
                name: "un",
                validator
            },
        ];

        // ✅ Use case: valid username
        // println!("{}",stages);

        assert!(p.run("<b>user_1</b>").is_ok());

        // ❌ Use case: invalid username
        assert!(p.run("<b>1bad</b>").is_err());
    }
}
