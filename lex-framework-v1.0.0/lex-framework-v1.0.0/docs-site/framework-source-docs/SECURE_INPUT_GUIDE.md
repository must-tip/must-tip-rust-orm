> **Historical documentation.** These pre-audit notes may describe obsolete APIs or ownership. Use the [current developer documentation](README.md) and executable acceptance examples for the supported contract.

# Secure Input Guide

`secure_input` provides boundary normalization, validation, sanitization, and context-specific output encoding.

## Core principle

Sanitization is not SQL injection protection. ORM/database code must still use bind parameters.

Likewise, one output encoder is not valid for every sink.

## Validation

Available helpers include username, URL, UUID, IP, hex, phone, generic input, filename, email, and password validation.

```rust
use secure_input::validate;

validate::validate_username("alice_01")?;
validate::validate_url("https://example.com")?;
validate::validate_uuid("550e8400-e29b-41d4-a716-446655440000")?;
```

## Password strength

`validation_pluse` exposes password strength evaluation and password/email validators.

## Sanitization

Use the narrowest transformation required by the domain. Over-sanitizing can corrupt legitimate data.

Common functions include Unicode normalization, whitespace collapse, HTML sanitization, numeric filtering, special-character removal, and defensive payload rejection.

## Output encoding

Use sink-specific functions from `encoding`:

```text
sanitize_html_whitelist
escape_json_string
escape_xml_text
escape_js_str
escape_css_string
encode_url_param
build_query_string
```

Never insert raw untrusted text into JavaScript/CSS/XML/HTML/URL output without the appropriate encoder.

## Pipelines

Compose boundary processing with `Pipeline` and `SanitizerStage`, `ValidatorStage`, and `FilterStage` when a deterministic sequence is required.
