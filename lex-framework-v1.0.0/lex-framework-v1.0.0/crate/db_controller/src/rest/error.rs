use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestError {
    pub code: String,
    pub detail: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, Vec<String>>,
}

impl RestError {
    pub fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: detail.into(),
            fields: BTreeMap::new(),
        }
    }

    pub fn validation(fields: BTreeMap<String, Vec<String>>) -> Self {
        Self {
            code: "validation_error".into(),
            detail: "request validation failed".into(),
            fields,
        }
    }
}

impl fmt::Display for RestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for RestError {}

impl From<ModelError> for RestError {
    fn from(error: ModelError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}
