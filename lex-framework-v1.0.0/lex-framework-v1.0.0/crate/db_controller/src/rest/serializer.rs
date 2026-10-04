use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerializerMode {
    Create,
    Update,
    PartialUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializerFieldPolicy {
    pub read_only: bool,
    pub write_only: bool,
    pub required_on_create: bool,
    pub allow_null: bool,
}

#[derive(Debug, Clone)]
pub struct SerializerPolicy {
    fields: BTreeMap<String, SerializerFieldPolicy>,
    schema: BTreeMap<String, crate::ColumnSpec>,
    deny_unknown_fields: bool,
}

impl SerializerPolicy {
    pub fn for_model<T: Entity>() -> Self {
        let schema = T::schema();
        let mut fields = BTreeMap::new();
        for (name, spec) in &schema {
            let framework_owned = spec.primary_key
                || matches!(
                    name.as_str(),
                    "created_at"
                        | "updated_at"
                        | "owner_id"
                        | "is_admin"
                        | "is_global_admin"
                        | "is_tenant_admin"
                        | "role"
                        | "role_id"
                        | "membership_id"
                        | "permission_id"
                        | "permissions"
                )
                || T::tenant_column().is_some_and(|column| column == name.as_str())
                || T::soft_delete_column().is_some_and(|column| column == name.as_str())
                || T::version_column().is_some_and(|column| column == name.as_str());
            fields.insert(
                name.clone(),
                SerializerFieldPolicy {
                    read_only: framework_owned,
                    write_only: spec.encrypted && !framework_owned,
                    required_on_create: !framework_owned
                        && !spec.nullable
                        && spec.default.is_none(),
                    allow_null: spec.nullable,
                },
            );
        }
        for relation in T::relations() {
            fields
                .entry(relation.field_name.to_owned())
                .or_insert(SerializerFieldPolicy {
                    read_only: true,
                    write_only: false,
                    required_on_create: false,
                    allow_null: relation.nullable,
                });
        }
        Self {
            fields,
            schema: schema.into_iter().collect(),
            deny_unknown_fields: true,
        }
    }

    pub fn set_field(mut self, name: impl Into<String>, policy: SerializerFieldPolicy) -> Self {
        self.fields.insert(name.into(), policy);
        self
    }

    /// Restrict client mutations to an explicit set of already writable model fields.
    /// Unknown or read-only selections are errors; security defaults are never implicitly enabled.
    pub fn allow_write_fields(mut self, names: &[&str]) -> Result<Self, RestError> {
        for name in names {
            match self.fields.get(*name) {
                Some(policy) if !policy.read_only => {}
                Some(_) => {
                    return Err(RestError::new(
                        "read_only_write_field",
                        format!("field {name} is read-only"),
                    ));
                }
                None => {
                    return Err(RestError::new(
                        "unknown_write_field",
                        format!("field {name} is not a model field"),
                    ));
                }
            }
        }
        for (name, policy) in &mut self.fields {
            if !names.contains(&name.as_str()) {
                policy.read_only = true;
                policy.required_on_create = false;
            }
        }
        Ok(self)
    }

    pub fn deny_unknown_fields(mut self, deny: bool) -> Self {
        self.deny_unknown_fields = deny;
        self
    }

    pub fn validate_input(
        &self,
        input: &Value,
        mode: SerializerMode,
    ) -> Result<Map<String, Value>, RestError> {
        let Some(object) = input.as_object() else {
            return Err(RestError::new(
                "invalid_payload",
                "request body must be a JSON object",
            ));
        };

        if object.len() > 512 {
            return Err(RestError::new(
                "payload_too_complex",
                "request body contains more than 512 top-level fields",
            ));
        }

        let mut errors: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut validated = Map::new();

        if mode == SerializerMode::Create {
            for (name, policy) in &self.fields {
                if policy.required_on_create && !object.contains_key(name) {
                    errors
                        .entry(name.clone())
                        .or_default()
                        .push("This field is required.".into());
                }
            }
        }

        for (name, value) in object {
            let Some(policy) = self.fields.get(name) else {
                if self.deny_unknown_fields {
                    errors
                        .entry(name.clone())
                        .or_default()
                        .push("Unknown field.".into());
                }
                continue;
            };
            if policy.read_only {
                errors
                    .entry(name.clone())
                    .or_default()
                    .push("This field is read-only.".into());
                continue;
            }
            if value.is_null() && !policy.allow_null {
                errors
                    .entry(name.clone())
                    .or_default()
                    .push("Null is not allowed.".into());
                continue;
            }
            if !value.is_null()
                && let Some(spec) = self.schema.get(name)
                && let Err(error) = DbValue::from_json_for_column(spec, value)
            {
                errors
                    .entry(name.clone())
                    .or_default()
                    .push(error.to_string());
                continue;
            }
            validated.insert(name.clone(), value.clone());
        }

        if errors.is_empty() {
            Ok(validated)
        } else {
            Err(RestError::validation(errors))
        }
    }

    pub fn validate_input_authorized<T, A>(
        &self,
        input: &Value,
        mode: SerializerMode,
        access: &A,
    ) -> Result<Map<String, Value>, RestError>
    where
        T: Entity,
        A: RestAccessPolicy<T> + ?Sized,
    {
        let validated = self.validate_input(input, mode)?;
        let action = match mode {
            SerializerMode::Create => RestAction::Create,
            SerializerMode::Update => RestAction::Update,
            SerializerMode::PartialUpdate => RestAction::PartialUpdate,
        };
        access.authorize_write(action, &validated)?;
        Ok(validated)
    }

    /// Run model-level validation after transport validation and before persistence.
    pub fn validate_model<T: Entity>(&self, value: &T) -> Result<(), RestError> {
        value
            .validate_model()
            .map_err(|detail| RestError::new("model_validation_error", detail))
    }

    pub fn serialize<T: Serialize>(&self, value: &T) -> Result<Value, RestError> {
        let mut value = serde_json::to_value(value)
            .map_err(|error| RestError::new("serialization_error", error.to_string()))?;
        if let Value::Object(object) = &mut value {
            object.retain(|name, _| {
                self.fields
                    .get(name)
                    .is_some_and(|policy| !policy.write_only)
            });
        }
        Ok(value)
    }

    pub fn deserialize<T: DeserializeOwned + Entity>(
        &self,
        input: &Value,
        mode: SerializerMode,
    ) -> Result<T, RestError> {
        if mode == SerializerMode::PartialUpdate {
            return Err(RestError::new(
                "partial_deserialization_requires_instance",
                "partial updates return validated fields and must be applied to an existing model instance",
            ));
        }
        let validated = self.validate_input(input, mode)?;
        if let Some(field) = validated.keys().find(|name| {
            self.fields
                .get(*name)
                .is_some_and(|policy| policy.write_only)
        }) {
            return Err(RestError::new(
                "sensitive_field_requires_transform",
                format!(
                    "field {field} requires an application-owned hashing/encryption transform before model construction"
                ),
            ));
        }
        let model: T = serde_json::from_value(Value::Object(validated))
            .map_err(|error| RestError::new("deserialization_error", error.to_string()))?;
        self.validate_model(&model)?;
        Ok(model)
    }
}
