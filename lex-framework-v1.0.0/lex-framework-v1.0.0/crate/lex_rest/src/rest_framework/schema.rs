use http::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

use crate::rest_framework::exceptions::{ApiError, ApiResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecuritySchemeMetadata {
    pub name: String,
    pub scheme: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationMetadata {
    pub operation_id: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub deprecated: bool,
    pub security: Vec<String>,
    pub request_schema: Option<Value>,
    pub response_schema: Option<Value>,
}

impl OperationMetadata {
    pub fn validate(&self) -> ApiResult<()> {
        if self.operation_id.is_empty()
            || self.operation_id.len() > 255
            || !self
                .operation_id
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        {
            return Err(ApiError::configuration("invalid OpenAPI operationId"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ApiSchema {
    title: String,
    version: String,
    paths: BTreeMap<String, BTreeMap<String, OperationMetadata>>,
    operation_ids: std::collections::BTreeSet<String>,
    security_schemes: BTreeMap<String, Value>,
}

impl ApiSchema {
    pub fn new(title: impl Into<String>, version: impl Into<String>) -> ApiResult<Self> {
        let title = title.into();
        let version = version.into();
        if title.trim().is_empty() || version.trim().is_empty() {
            return Err(ApiError::configuration(
                "API title and version are required",
            ));
        }
        Ok(Self {
            title,
            version,
            paths: BTreeMap::new(),
            operation_ids: std::collections::BTreeSet::new(),
            security_schemes: BTreeMap::new(),
        })
    }

    pub fn add_security_scheme(&mut self, metadata: SecuritySchemeMetadata) -> ApiResult<()> {
        if metadata.name.is_empty()
            || metadata.name.len() > 128
            || !metadata
                .name
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        {
            return Err(ApiError::configuration("invalid security scheme name"));
        }
        if self.security_schemes.contains_key(&metadata.name) {
            return Err(ApiError::configuration("duplicate security scheme"));
        }
        self.security_schemes.insert(metadata.name, metadata.scheme);
        Ok(())
    }

    pub fn add_operation(
        &mut self,
        path: impl Into<String>,
        method: &Method,
        metadata: OperationMetadata,
    ) -> ApiResult<()> {
        metadata.validate()?;
        let path = path.into();
        if !path.starts_with('/') || path.len() > 2048 || path.contains('?') || path.contains('#') {
            return Err(ApiError::configuration("invalid OpenAPI path"));
        }
        let method = method.as_str().to_ascii_lowercase();
        if !matches!(
            method.as_str(),
            "get" | "post" | "put" | "patch" | "delete" | "head" | "options"
        ) {
            return Err(ApiError::configuration("unsupported OpenAPI HTTP method"));
        }
        if self
            .paths
            .get(&path)
            .is_some_and(|operations| operations.contains_key(&method))
        {
            return Err(ApiError::configuration("duplicate OpenAPI operation"));
        }
        if self.operation_ids.contains(&metadata.operation_id) {
            return Err(ApiError::configuration("duplicate OpenAPI operationId"));
        }
        self.operation_ids.insert(metadata.operation_id.clone());
        self.paths.entry(path).or_default().insert(method, metadata);
        Ok(())
    }

    /// OpenAPI 3.2.0 document for the registered HTTP operations.
    pub fn document(&self) -> Value {
        let mut paths = Map::new();
        for (path, operations) in &self.paths {
            let mut path_item = Map::new();
            for (method, metadata) in operations {
                let security = metadata
                    .security
                    .iter()
                    .map(|name| {
                        let mut requirement = Map::new();
                        requirement.insert(name.clone(), json!([]));
                        Value::Object(requirement)
                    })
                    .collect::<Vec<_>>();

                let mut operation = Map::new();
                operation.insert("operationId".to_owned(), json!(metadata.operation_id));
                operation.insert("summary".to_owned(), json!(metadata.summary));
                operation.insert("description".to_owned(), json!(metadata.description));
                operation.insert("tags".to_owned(), json!(metadata.tags));
                operation.insert("deprecated".to_owned(), json!(metadata.deprecated));
                operation.insert("security".to_owned(), json!(security));

                let parameters = path_parameters(path);
                if !parameters.is_empty() {
                    operation.insert("parameters".to_owned(), Value::Array(parameters));
                }
                if let Some(schema) = metadata.request_schema.as_ref() {
                    operation.insert(
                        "requestBody".to_owned(),
                        json!({
                            "required": true,
                            "content": {"application/json": {"schema": schema}}
                        }),
                    );
                }

                let (success_code, success_description) = success_response(method);
                let mut responses = Map::new();
                let mut success = Map::new();
                success.insert("description".to_owned(), json!(success_description));
                if success_code != "204"
                    && let Some(schema) = metadata.response_schema.as_ref()
                {
                    success.insert(
                        "content".to_owned(),
                        json!({"application/json": {"schema": schema}}),
                    );
                }
                responses.insert(success_code.to_owned(), Value::Object(success));
                for (status, description) in [
                    ("400", "Invalid request"),
                    ("401", "Authentication required"),
                    ("403", "Permission denied"),
                    ("409", "Conflict"),
                    ("429", "Rate limit exceeded"),
                    ("500", "Internal server error"),
                ] {
                    responses.insert(
                        status.to_owned(),
                        json!({
                            "description": description,
                            "content": {
                                "application/json": {
                                    "schema": {"$ref": "#/components/schemas/Error"}
                                }
                            }
                        }),
                    );
                }
                operation.insert("responses".to_owned(), Value::Object(responses));
                path_item.insert(method.clone(), Value::Object(operation));
            }
            paths.insert(path.clone(), Value::Object(path_item));
        }
        json!({
            "openapi": "3.2.0",
            "info": {"title": self.title, "version": self.version},
            "paths": paths,
            "components": {
                "securitySchemes": self.security_schemes,
                "schemas": {
                    "Error": {
                        "type": "object",
                        "required": ["code", "detail", "request_id"],
                        "properties": {
                            "code": {"type": "string"},
                            "detail": {"type": "string"},
                            "request_id": {"type": "string", "format": "uuid"}
                        }
                    }
                }
            }
        })
    }
}

fn success_response(method: &str) -> (&'static str, &'static str) {
    match method {
        "post" => ("201", "Created"),
        "delete" => ("204", "Deleted"),
        _ => ("200", "Success"),
    }
}

fn path_parameters(path: &str) -> Vec<Value> {
    path.split('/')
        .filter_map(|segment| {
            segment
                .strip_prefix('{')
                .and_then(|value| value.strip_suffix('}'))
        })
        .filter(|name| !name.is_empty())
        .map(|name| {
            json!({
                "name": name,
                "in": "path",
                "required": true,
                "schema": {"type": "string"}
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(id: &str) -> OperationMetadata {
        OperationMetadata {
            operation_id: id.to_owned(),
            summary: None,
            description: None,
            tags: vec!["users".to_owned()],
            deprecated: false,
            security: Vec::new(),
            request_schema: Some(json!({"type": "object"})),
            response_schema: Some(json!({"type": "object"})),
        }
    }

    #[test]
    fn schema_declares_path_parameters_and_http_success_codes() {
        let mut schema = ApiSchema::new("Test", "1").expect("schema");
        schema
            .add_operation("/users/{id}", &Method::DELETE, metadata("users.delete"))
            .expect("operation");
        let document = schema.document();
        assert!(document["paths"]["/users/{id}"]["delete"]["parameters"].is_array());
        assert!(document["paths"]["/users/{id}"]["delete"]["responses"]["204"].is_object());
        assert!(
            document["paths"]["/users/{id}"]["delete"]["responses"]["204"]["content"].is_null()
        );
    }

    #[test]
    fn duplicate_operation_ids_are_rejected() {
        let mut schema = ApiSchema::new("Test", "1").expect("schema");
        schema
            .add_operation("/users", &Method::GET, metadata("users.list"))
            .expect("operation");
        assert!(
            schema
                .add_operation("/accounts", &Method::GET, metadata("users.list"))
                .is_err()
        );
    }
}
