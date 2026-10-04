//! Strongly typed model field wrappers used by `db_controller` and the `orm` derive macro.
//!
//! These wrappers keep domain intent explicit. Validation of untrusted input belongs at API
//! boundaries; persistence metadata is supplied through `Entity` and the derive macro.

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use proc_macro2::TokenStream;
use quote::quote;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::marker::PhantomData;
use syn::Type;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForeignKey<T> {
    pub id: i32,
    #[serde(skip)]
    pub _phantom: PhantomData<T>,
}

impl<T> ForeignKey<T> {
    pub fn new(id: i32) -> Self {
        Self {
            id,
            _phantom: PhantomData,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.id > 0
    }

    pub fn into_option(self) -> Option<i32> {
        Some(self.id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManyToMany<T> {
    pub related_ids: Vec<i32>,
    #[serde(skip)]
    pub _phantom: PhantomData<T>,
}

impl<T> Default for ManyToMany<T> {
    fn default() -> Self {
        Self {
            related_ids: Vec::new(),
            _phantom: PhantomData,
        }
    }
}

impl<T> ManyToMany<T> {
    pub fn new(related_ids: Vec<i32>) -> Self {
        Self {
            related_ids,
            _phantom: PhantomData,
        }
    }

    pub fn contains(&self, id: i32) -> bool {
        self.related_ids.contains(&id)
    }

    pub fn is_empty(&self) -> bool {
        self.related_ids.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct UUIDField(pub Uuid);

impl UUIDField {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Compatibility shorthand for the implemented `FromStr` trait.
    #[allow(
        clippy::should_implement_trait,
        reason = "preserve the existing inherent parsing API alongside FromStr"
    )]
    pub fn from_str(value: &str) -> Result<Self, uuid::Error> {
        value.parse()
    }

    pub fn as_str(&self) -> String {
        self.0.to_string()
    }
}

impl std::str::FromStr for UUIDField {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

impl Default for UUIDField {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct DateTimeField(pub DateTime<Utc>);

impl DateTimeField {
    pub fn now() -> Self {
        Self(Utc::now())
    }

    /// Compatibility shorthand for the implemented `FromStr` trait.
    #[allow(
        clippy::should_implement_trait,
        reason = "preserve the existing inherent parsing API alongside FromStr"
    )]
    pub fn from_str(value: &str) -> Result<Self, chrono::ParseError> {
        value.parse()
    }

    pub fn to_rfc3339(&self) -> String {
        self.0.to_rfc3339()
    }
}

impl std::str::FromStr for DateTimeField {
    type Err = chrono::ParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        DateTime::parse_from_rfc3339(value).map(|value| Self(value.with_timezone(&Utc)))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct DecimalField(pub Decimal);

impl DecimalField {
    pub fn new(value: Decimal) -> Self {
        Self(value)
    }

    /// Compatibility shorthand for the implemented `FromStr` trait.
    #[allow(
        clippy::should_implement_trait,
        reason = "preserve the existing inherent parsing API alongside FromStr"
    )]
    pub fn from_str(value: &str) -> Result<Self, rust_decimal::Error> {
        value.parse()
    }

    pub fn value(&self) -> Decimal {
        self.0
    }
}

impl std::str::FromStr for DecimalField {
    type Err = rust_decimal::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse::<Decimal>().map(Self)
    }
}

#[derive(Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct PasswordField(pub String);

impl std::fmt::Debug for PasswordField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("PasswordField")
            .field(&"[REDACTED]")
            .finish()
    }
}

impl PasswordField {
    /// Hash a password with Argon2id and a cryptographically random salt.
    pub fn hash(password: impl AsRef<[u8]>) -> Result<Self, argon2::password_hash::Error> {
        let salt = SaltString::generate(&mut OsRng);
        let encoded = Argon2::default()
            .hash_password(password.as_ref(), &salt)?
            .to_string();
        Ok(Self(encoded))
    }

    /// Verify a candidate against a PHC-formatted Argon2 password hash.
    pub fn verify(&self, password: impl AsRef<[u8]>) -> Result<bool, argon2::password_hash::Error> {
        let parsed = PasswordHash::new(&self.0)?;
        match Argon2::default().verify_password(password.as_ref(), &parsed) {
            Ok(()) => Ok(true),
            Err(argon2::password_hash::Error::Password) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub struct JsonField<T>(pub T);

impl<T> JsonField<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn inner(&self) -> &T {
        &self.0
    }

    pub fn serialize_json(&self) -> Result<String, serde_json::Error>
    where
        T: Serialize,
    {
        serde_json::to_string(&self.0)
    }

    pub fn deserialize_json(json: &str) -> Result<Self, serde_json::Error>
    where
        T: DeserializeOwned,
    {
        serde_json::from_str(json).map(Self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct BooleanField(pub bool);

impl BooleanField {
    pub fn new(value: bool) -> Self {
        Self(value)
    }

    pub fn is_true(&self) -> bool {
        self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct IntegerField(pub i32);

impl IntegerField {
    pub fn new(value: i32) -> Self {
        Self(value)
    }

    pub fn value(&self) -> i32 {
        self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct FloatField(pub f64);

impl FloatField {
    pub fn new(value: f64) -> Self {
        Self(value)
    }

    pub fn value(&self) -> f64 {
        self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct TextField(pub String);

impl TextField {
    pub fn new<S: Into<String>>(value: S) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct SlugField(pub String);

impl SlugField {
    pub fn new<S: Into<String>>(value: S) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_valid_slug(&self) -> bool {
        !self.0.is_empty()
            && self
                .0
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    }
}

/// Canonical raw byte field used by all media aliases.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, sqlx::Type)]
#[sqlx(transparent, no_pg_array)]
pub struct BinaryField(pub Vec<u8>);

impl BinaryField {
    pub fn new(value: Vec<u8>) -> Self {
        Self(value)
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn to_base64(&self) -> String {
        STANDARD.encode(&self.0)
    }

    pub fn from_base64(value: &str) -> Result<Self, base64::DecodeError> {
        STANDARD.decode(value).map(Self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
pub struct ChoiceField<T>
where
    T: Clone + PartialEq,
{
    pub value: T,
    pub choices: Vec<T>,
}

impl<T> ChoiceField<T>
where
    T: Clone + PartialEq + std::fmt::Debug,
{
    pub fn try_new(value: T, choices: Vec<T>) -> Result<Self, String> {
        if !choices.contains(&value) {
            return Err(format!(
                "Invalid choice `{:?}`; expected one of {:?}",
                value, choices
            ));
        }
        Ok(Self { value, choices })
    }

    /// Retained for backward compatibility with trusted/domain-controlled construction.
    pub fn new(value: T, choices: Vec<T>) -> Self {
        Self::try_new(value, choices).unwrap_or_else(|message| panic!("{message}"))
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn is_valid(&self) -> bool {
        self.choices.contains(&self.value)
    }
}

pub fn map_type_to_wrapper(ty: &Type) -> TokenStream {
    let normalized = quote!(#ty).to_string().replace(' ', "");
    match normalized.as_str() {
        "Uuid" | "uuid::Uuid" => quote!(UUIDField),
        "DateTime<Utc>" | "chrono::DateTime<Utc>" => quote!(DateTimeField),
        "Decimal" | "rust_decimal::Decimal" => quote!(DecimalField),
        value if value.starts_with("JsonField") => quote!(#ty),
        "bool" => quote!(BooleanField),
        "i32" => quote!(IntegerField),
        "f64" => quote!(FloatField),
        "String" => quote!(TextField),
        "SlugField" => quote!(SlugField),
        "Vec<u8>" | "BinaryField" => quote!(BinaryField),
        "ImageField" => quote!(ImageField),
        "AudioField" => quote!(AudioField),
        "VideoField" => quote!(VideoField),
        "PdfField" => quote!(PdfField),
        "PasswordField" => quote!(PasswordField),
        "DocumentField" => quote!(DocumentField),
        value if value.starts_with("ForeignKey") => quote!(#ty),
        value if value.starts_with("ManyToMany") => quote!(#ty),
        other => syn::parse_str::<Type>(other)
            .map(|parsed| quote!(#parsed))
            .unwrap_or_else(|_| quote!(#ty)),
    }
}

pub fn map_type_to_sql(ty: &Type) -> String {
    let mut normalized = quote!(#ty).to_string();
    normalized.retain(|ch| !ch.is_whitespace());

    if let Some(inner) = normalized
        .strip_prefix("Option<")
        .and_then(|rest| rest.strip_suffix('>'))
        && let Ok(inner_type) = syn::parse_str::<Type>(inner)
    {
        return map_type_to_sql(&inner_type);
    }

    match normalized.as_str() {
        "Uuid" | "uuid::Uuid" | "UUIDField" => "UUID".into(),
        "DateTime<Utc>" | "chrono::DateTime<Utc>" | "DateTimeField" => "TIMESTAMPTZ".into(),
        "Decimal" | "rust_decimal::Decimal" | "DecimalField" => "NUMERIC".into(),
        "PasswordField" => "TEXT".into(),
        "bool" | "BooleanField" => "BOOLEAN".into(),
        "i32" | "IntegerField" => "INTEGER".into(),
        "i64" => "BIGINT".into(),
        "f32" => "REAL".into(),
        "f64" | "FloatField" => "DOUBLE PRECISION".into(),
        "String" | "TextField" | "SlugField" => "TEXT".into(),
        value if value.starts_with("JsonField") => "JSONB".into(),
        value if value.starts_with("ChoiceField") => "TEXT".into(),
        "Vec<u8>" | "BinaryField" | "ImageField" | "AudioField" | "VideoField" | "PdfField"
        | "DocumentField" => "BYTEA".into(),
        value if value.starts_with("ForeignKey") => "INTEGER".into(),
        value if value.starts_with("ManyToMany") => "JSONB".into(),
        _ => "TEXT".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;
    use serde_json::{Value, json};

    #[test]
    fn relationship_wrappers_work() {
        let fk = ForeignKey::<()>::new(42);
        assert!(fk.is_valid());
        assert_eq!(fk.into_option(), Some(42));

        let relations = ManyToMany::<()>::new(vec![1, 2, 3]);
        assert!(relations.contains(2));
        assert!(!relations.is_empty());
        assert!(ManyToMany::<()>::default().is_empty());
    }

    #[test]
    fn scalar_wrappers_roundtrip() {
        let uuid = UUIDField::new();
        let parsed = UUIDField::from_str(&uuid.as_str())
            .unwrap_or_else(|error| panic!("uuid parse failed: {error}"));
        assert_eq!(uuid.0, parsed.0);

        let now = Utc::now();
        let parsed_time = DateTimeField::from_str(&DateTimeField(now).to_rfc3339())
            .unwrap_or_else(|error| panic!("datetime parse failed: {error}"));
        assert_eq!(parsed_time.0.timestamp(), now.timestamp());

        let decimal = Decimal::new(12345, 2);
        assert_eq!(DecimalField::new(decimal).value(), decimal);
        assert_eq!(
            DecimalField::from_str("99.99")
                .unwrap_or_else(|error| panic!("decimal parse failed: {error}"))
                .value(),
            Decimal::new(9999, 2)
        );
        assert!(DecimalField::from_str("NaN").is_err());

        assert!(BooleanField::new(true).is_true());
        assert_eq!(IntegerField::new(123).value(), 123);
        assert!((FloatField::new(3.5).value() - 3.5).abs() < f64::EPSILON);
        assert_eq!(TextField::new(" hello ").as_str(), " hello ");
        assert!(TextField::new("   ").is_empty());
        assert!(SlugField::new("a-b-1").is_valid_slug());
        assert!(!SlugField::new("Bad_Slug!").is_valid_slug());
    }

    #[test]
    fn json_field_roundtrips() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct Example {
            x: i32,
            y: String,
        }

        let field = JsonField::new(Example {
            x: 7,
            y: "hi".into(),
        });
        let encoded = field
            .serialize_json()
            .unwrap_or_else(|error| panic!("serialize failed: {error}"));
        let decoded: JsonField<Example> = JsonField::deserialize_json(&encoded)
            .unwrap_or_else(|error| panic!("deserialize failed: {error}"));
        assert_eq!(decoded.inner(), field.inner());
    }

    #[test]
    fn password_hash_roundtrip_and_debug_redaction() {
        let password = PasswordField::hash("Correct-Horse-Battery-Staple-42")
            .unwrap_or_else(|error| panic!("hash failed: {error}"));
        assert!(password.as_str().starts_with("$argon2id$"));
        assert!(matches!(
            password.verify("Correct-Horse-Battery-Staple-42"),
            Ok(true)
        ));
        assert!(matches!(password.verify("wrong-password"), Ok(false)));
        assert!(!format!("{password:?}").contains(password.as_str()));
    }

    #[test]
    fn choice_field_supports_fallible_external_input() {
        let field = ChoiceField::try_new("green", vec!["red", "green", "blue"])
            .unwrap_or_else(|error| panic!("choice failed: {error}"));
        assert_eq!(field.value(), &"green");
        assert!(field.is_valid());
        assert!(ChoiceField::try_new(1, vec![2, 3]).is_err());
    }

    #[test]
    #[should_panic(expected = "Invalid choice")]
    fn choice_field_legacy_constructor_fails_fast() {
        let _ = ChoiceField::new(1, vec![2, 3]);
    }

    #[test]
    fn binary_field_base64_roundtrip() {
        let data = vec![0x01, 0x02, 0xFF];
        let binary = BinaryField::new(data.clone());
        let encoded = binary.to_base64();
        let decoded = BinaryField::from_base64(&encoded)
            .unwrap_or_else(|error| panic!("base64 decode failed: {error}"));
        assert_eq!(decoded.bytes(), data.as_slice());
        assert!(BinaryField::from_base64("!!!").is_err());
    }

    #[test]
    fn wrappers_serialize_without_hidden_runtime_state() {
        let value = json!({
            "fk": ForeignKey::<()>::new(5),
            "mtm": ManyToMany::<()>::new(vec![1, 2]),
            "uuid": UUIDField::new().as_str(),
            "dt": DateTimeField::now().to_rfc3339(),
            "dec": DecimalField::new(Decimal::new(1, 0)),
            "b": BooleanField::new(false).0,
            "i": IntegerField::new(7).0,
            "f": FloatField::new(2.5).0,
            "txt": TextField::new("x").as_str(),
            "slug": SlugField::new("x-y").as_str(),
            "ch": ChoiceField::new("a", vec!["a", "b"]).value(),
            "bin": BinaryField::new(vec![1, 2, 3]).to_base64(),
        });
        let encoded = serde_json::to_string(&value)
            .unwrap_or_else(|error| panic!("serialize failed: {error}"));
        let decoded: Value = serde_json::from_str(&encoded)
            .unwrap_or_else(|error| panic!("deserialize failed: {error}"));
        assert_eq!(value, decoded);
    }
}
