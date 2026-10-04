use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaginationStyle {
    PageNumber,
    LimitOffset,
    Cursor {
        ordering_field: String,
        direction: SortDirection,
    },
}

#[derive(Clone)]
pub(super) struct CursorKeyRing {
    pub(super) active: Arc<[u8]>,
    pub(super) previous: Vec<Arc<[u8]>>,
}

impl fmt::Debug for CursorKeyRing {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CursorKeyRing")
            .field("active", &"[REDACTED]")
            .field("previous_key_count", &self.previous.len())
            .finish()
    }
}

impl CursorKeyRing {
    pub(super) fn new(active: Vec<u8>) -> Result<Self, RestError> {
        validate_cursor_key(&active)?;
        Ok(Self {
            active: Arc::from(active),
            previous: Vec::new(),
        })
    }

    pub(super) fn push_previous(&mut self, key: Vec<u8>) -> Result<(), RestError> {
        validate_cursor_key(&key)?;
        if self.previous.len() >= 3 {
            return Err(RestError::new(
                "invalid_cursor_key",
                "at most three previous cursor signing keys are supported",
            ));
        }
        self.previous.push(Arc::from(key));
        Ok(())
    }

    pub(super) fn sign(&self, payload: &[u8]) -> Result<Vec<u8>, RestError> {
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = HmacSha256::new_from_slice(&self.active)
            .map_err(|_| RestError::new("cursor_error", "invalid cursor signing key"))?;
        mac.update(payload);
        Ok(mac.finalize().into_bytes().to_vec())
    }

    pub(super) fn verify(&self, payload: &[u8], signature: &[u8]) -> bool {
        self.verify_key(&self.active, payload, signature)
            || self
                .previous
                .iter()
                .any(|key| self.verify_key(key, payload, signature))
    }

    pub(super) fn verify_key(&self, key: &[u8], payload: &[u8], signature: &[u8]) -> bool {
        type HmacSha256 = Hmac<Sha256>;
        let Ok(mut mac) = HmacSha256::new_from_slice(key) else {
            return false;
        };
        mac.update(payload);
        mac.verify_slice(signature).is_ok()
    }
}

pub(super) fn validate_cursor_key(key: &[u8]) -> Result<(), RestError> {
    if !(32..=1024).contains(&key.len()) {
        return Err(RestError::new(
            "invalid_cursor_key",
            "cursor signing keys must contain 32..=1024 bytes",
        ));
    }
    Ok(())
}

pub struct AppliedRestQuery<T, E> {
    pub queryset: QuerySet<T, E>,
    pub(super) cursor_page: Option<CursorPagePolicy>,
}

impl<T, E> AppliedRestQuery<T, E>
where
    T: Entity,
{
    pub fn cursor_page_policy(&self) -> Option<&CursorPagePolicy> {
        self.cursor_page.as_ref()
    }
}

#[derive(Debug, Clone)]
pub struct CursorPagePolicy {
    pub field: String,
    pub direction: SortDirection,
    pub page_size: u32,
    pub(super) keys: CursorKeyRing,
}

impl CursorPagePolicy {
    pub fn finalize<T: Entity>(&self, mut rows: Vec<T>) -> Result<CursorPage<T>, RestError> {
        let has_more = rows.len() > self.page_size as usize;
        if has_more {
            rows.truncate(self.page_size as usize);
        }
        let next = if has_more {
            rows.last()
                .map(|row| CursorToken::from_model(row, &self.field, self.direction))
                .transpose()?
                .map(|cursor| cursor.encode(&self.keys))
                .transpose()?
        } else {
            None
        };
        Ok(CursorPage {
            results: rows,
            next,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CursorPage<T> {
    pub results: Vec<T>,
    pub next: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct CursorToken {
    pub(super) version: u8,
    pub(super) model: String,
    pub(super) field: String,
    pub(super) direction: SortDirection,
    pub(super) value: String,
    pub(super) primary_key: String,
}

impl CursorToken {
    pub(super) const VERSION: u8 = 1;

    pub(super) fn from_model<T: Entity>(
        model: &T,
        field: &str,
        direction: SortDirection,
    ) -> Result<Self, RestError> {
        let map = model.to_map();
        let value = map.get(field).ok_or_else(|| {
            RestError::new(
                "cursor_error",
                format!("model did not emit cursor field {field}"),
            )
        })?;
        let primary_key = map
            .get(T::primary_key())
            .ok_or_else(|| RestError::new("cursor_error", "model did not emit primary key"))?;
        Ok(Self {
            version: Self::VERSION,
            model: T::table_name().to_owned(),
            field: field.to_owned(),
            direction,
            value: scalar_cursor_value(value)?,
            primary_key: scalar_cursor_value(primary_key)?,
        })
    }

    pub(super) fn encode(&self, keys: &CursorKeyRing) -> Result<String, RestError> {
        let payload = serde_json::to_vec(self)
            .map_err(|error| RestError::new("cursor_error", error.to_string()))?;
        let signature = keys.sign(&payload)?;
        Ok(format!(
            "v1.{}.{}",
            URL_SAFE_NO_PAD.encode(payload),
            URL_SAFE_NO_PAD.encode(signature)
        ))
    }

    pub(super) fn decode(value: &str, keys: Option<&CursorKeyRing>) -> Result<Self, RestError> {
        if value.len() > 4096 {
            return Err(RestError::new("invalid_cursor", "cursor is too large"));
        }
        let keys = keys.ok_or_else(|| {
            RestError::new(
                "invalid_policy",
                "cursor pagination requires an HMAC signing key",
            )
        })?;
        let mut parts = value.split('.');
        if parts.next() != Some("v1") {
            return Err(RestError::new(
                "invalid_cursor",
                "unsupported cursor envelope version",
            ));
        }
        let payload = parts
            .next()
            .ok_or_else(|| RestError::new("invalid_cursor", "cursor payload is missing"))?;
        let signature = parts
            .next()
            .ok_or_else(|| RestError::new("invalid_cursor", "cursor signature is missing"))?;
        if parts.next().is_some() {
            return Err(RestError::new(
                "invalid_cursor",
                "cursor has an invalid envelope",
            ));
        }
        let payload = URL_SAFE_NO_PAD.decode(payload).map_err(|_| {
            RestError::new("invalid_cursor", "cursor payload is not valid base64url")
        })?;
        let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| {
            RestError::new("invalid_cursor", "cursor signature is not valid base64url")
        })?;
        if !keys.verify(&payload, &signature) {
            return Err(RestError::new(
                "invalid_cursor",
                "cursor signature verification failed",
            ));
        }
        let cursor: Self = serde_json::from_slice(&payload)
            .map_err(|_| RestError::new("invalid_cursor", "cursor payload is invalid"))?;
        if cursor.version != Self::VERSION {
            return Err(RestError::new(
                "invalid_cursor",
                "unsupported cursor version",
            ));
        }
        Ok(cursor)
    }
}
