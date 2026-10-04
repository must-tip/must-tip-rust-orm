use http::Method;
use percent_encoding::percent_decode_str;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use crate::rest_framework::{
    exceptions::{ApiError, ApiResult},
    schema::ApiSchema,
    views::APIView,
    websocket::WebSocketView,
};

#[derive(Clone)]
pub enum RouteEndpoint {
    Http(Arc<dyn APIView>),
    WebSocket(Arc<dyn WebSocketView>),
}

impl RouteEndpoint {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Http(view) => view.name(),
            Self::WebSocket(view) => view.name(),
        }
    }
}

#[derive(Clone)]
pub struct RouteSpec {
    pub path: String,
    pub name: String,
    pub endpoint: RouteEndpoint,
    pub tags: Vec<String>,
}

impl RouteSpec {
    pub fn http(path: impl Into<String>, name: impl Into<String>, view: Arc<dyn APIView>) -> Self {
        Self {
            path: path.into(),
            name: name.into(),
            endpoint: RouteEndpoint::Http(view),
            tags: Vec::new(),
        }
    }

    pub fn websocket(
        path: impl Into<String>,
        name: impl Into<String>,
        view: Arc<dyn WebSocketView>,
    ) -> Self {
        Self {
            path: path.into(),
            name: name.into(),
            endpoint: RouteEndpoint::WebSocket(view),
            tags: Vec::new(),
        }
    }

    pub fn tags(mut self, tags: impl IntoIterator<Item = String>) -> Self {
        self.tags = tags.into_iter().collect();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Static(String),
    Parameter(String),
}

#[derive(Clone)]
struct CompiledRoute {
    spec: RouteSpec,
    segments: Vec<Segment>,
    methods: BTreeSet<String>,
}

#[derive(Clone)]
pub struct ResolvedRoute {
    pub spec: RouteSpec,
    pub path_params: BTreeMap<String, String>,
}

#[derive(Clone, Default)]
pub struct RouteTable {
    routes: Vec<CompiledRoute>,
}

impl RouteTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_many(&mut self, specs: impl IntoIterator<Item = RouteSpec>) -> ApiResult<()> {
        for spec in specs {
            self.register(spec)?;
        }
        Ok(())
    }

    pub fn register(&mut self, spec: RouteSpec) -> ApiResult<()> {
        if spec.name.is_empty() || spec.name.len() > 255 {
            return Err(ApiError::configuration("invalid route name"));
        }
        if self.routes.iter().any(|route| route.spec.name == spec.name) {
            return Err(ApiError::configuration("duplicate route name"));
        }
        let segments = compile_path(&spec.path)?;
        let methods = match &spec.endpoint {
            RouteEndpoint::Http(view) => {
                let allowed = view.allowed_methods();
                if allowed.is_empty()
                    || allowed.iter().any(|method| {
                        method != Method::GET
                            && method != Method::POST
                            && method != Method::PUT
                            && method != Method::PATCH
                            && method != Method::DELETE
                    })
                {
                    return Err(ApiError::configuration(
                        "HTTP routes support explicit GET/POST/PUT/PATCH/DELETE methods only",
                    ));
                }
                allowed
                    .into_iter()
                    .map(|method| method.as_str().to_owned())
                    .collect()
            }
            RouteEndpoint::WebSocket(_) => BTreeSet::from([
                Method::GET.as_str().to_owned(),
                Method::CONNECT.as_str().to_owned(),
            ]),
        };
        if methods.is_empty() {
            return Err(ApiError::configuration("route has no allowed methods"));
        }
        for existing in &self.routes {
            if equivalent_shape(&segments, &existing.segments)
                && !methods.is_disjoint(&existing.methods)
            {
                return Err(ApiError::configuration(format!(
                    "route {} conflicts with {}",
                    spec.path, existing.spec.path
                )));
            }
        }
        self.routes.push(CompiledRoute {
            spec,
            segments,
            methods,
        });
        self.routes.sort_by(|left, right| {
            route_specificity(&right.segments)
                .cmp(&route_specificity(&left.segments))
                .then_with(|| left.spec.path.cmp(&right.spec.path))
        });
        Ok(())
    }

    pub fn resolve(&self, method: &Method, path: &str) -> ApiResult<ResolvedRoute> {
        let incoming = decode_path(path)?;
        let mut path_match_found = false;
        let mut allowed_methods = BTreeSet::new();
        for route in &self.routes {
            if route.segments.len() != incoming.len() {
                continue;
            }
            let Some(path_params) = match_segments(&route.segments, &incoming) else {
                continue;
            };
            path_match_found = true;
            allowed_methods.extend(route.methods.iter().cloned());
            if route.methods.contains(Method::GET.as_str()) {
                allowed_methods.insert(Method::HEAD.as_str().to_owned());
            }
            allowed_methods.insert(Method::OPTIONS.as_str().to_owned());
            let method_allowed = route.methods.contains(method.as_str())
                || (method == Method::HEAD && route.methods.contains(Method::GET.as_str()))
                || method == Method::OPTIONS;
            if method_allowed {
                return Ok(ResolvedRoute {
                    spec: route.spec.clone(),
                    path_params,
                });
            }
        }
        if path_match_found {
            Err(ApiError::method_not_allowed(allowed_methods))
        } else {
            Err(ApiError::not_found())
        }
    }

    pub fn routes(&self) -> Vec<RouteSpec> {
        self.routes.iter().map(|route| route.spec.clone()).collect()
    }

    pub fn build_openapi(&self, title: &str, version: &str) -> ApiResult<ApiSchema> {
        let mut schema = ApiSchema::new(title, version)?;
        for route in &self.routes {
            let RouteEndpoint::Http(view) = &route.spec.endpoint else {
                continue;
            };
            for method in view.allowed_methods() {
                if let Some(mut metadata) = view.operation_metadata(&method) {
                    if metadata.tags.is_empty() {
                        metadata.tags = route.spec.tags.clone();
                    }
                    schema.add_operation(route.spec.path.clone(), &method, metadata)?;
                }
            }
        }
        Ok(schema)
    }
}

fn compile_path(path: &str) -> ApiResult<Vec<Segment>> {
    if !path.starts_with('/') || path.len() > 2048 || path.contains('?') || path.contains('#') {
        return Err(ApiError::configuration("invalid route path"));
    }
    if path != "/" && path.ends_with('/') {
        return Err(ApiError::configuration("route paths must not end with '/'"));
    }
    let mut parameter_names = BTreeSet::new();
    path.trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            if segment.starts_with('{') && segment.ends_with('}') {
                let name = &segment[1..segment.len() - 1];
                if name.is_empty()
                    || name.len() > 64
                    || !name.chars().enumerate().all(|(index, ch)| {
                        ch == '_'
                            || ch.is_ascii_alphanumeric() && (index > 0 || !ch.is_ascii_digit())
                    })
                    || !parameter_names.insert(name.to_owned())
                {
                    return Err(ApiError::configuration("invalid route parameter"));
                }
                Ok(Segment::Parameter(name.to_owned()))
            } else if segment.contains('{') || segment.contains('}') || segment.len() > 255 {
                Err(ApiError::configuration("invalid static route segment"))
            } else {
                Ok(Segment::Static(segment.to_owned()))
            }
        })
        .collect()
}

fn decode_path(path: &str) -> ApiResult<Vec<String>> {
    if !path.starts_with('/') || path.len() > 8192 {
        return Err(ApiError::bad_request(
            "invalid_path",
            "invalid request path",
        ));
    }
    path.trim_start_matches('/')
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let decoded = percent_decode_str(segment)
                .decode_utf8()
                .map_err(|_| ApiError::bad_request("invalid_path", "path is not valid UTF-8"))?
                .into_owned();
            if decoded.len() > 4096
                || decoded.contains('/')
                || decoded.contains('\\')
                || decoded.contains('\0')
                || decoded == "."
                || decoded == ".."
            {
                return Err(ApiError::bad_request("invalid_path", "unsafe path segment"));
            }
            Ok(decoded)
        })
        .collect()
}

fn match_segments(route: &[Segment], incoming: &[String]) -> Option<BTreeMap<String, String>> {
    let mut params = BTreeMap::new();
    for (expected, actual) in route.iter().zip(incoming) {
        match expected {
            Segment::Static(value) if value != actual => return None,
            Segment::Static(_) => {}
            Segment::Parameter(name) => {
                params.insert(name.clone(), actual.clone());
            }
        }
    }
    Some(params)
}

fn equivalent_shape(left: &[Segment], right: &[Segment]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| match (left, right) {
                (Segment::Static(left), Segment::Static(right)) => left == right,
                (Segment::Parameter(_), Segment::Parameter(_)) => true,
                _ => false,
            })
}

fn route_specificity(segments: &[Segment]) -> usize {
    segments
        .iter()
        .filter(|segment| matches!(segment, Segment::Static(_)))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rest_framework::{
        request::{ApiRequest, RequestContext},
        response::ApiResponse,
        views::ViewServices,
        websocket::{WebSocketSession, WebSocketView},
    };
    use async_trait::async_trait;

    struct SocketView;
    #[async_trait]
    impl WebSocketView for SocketView {
        fn name(&self) -> &'static str {
            "socket"
        }

        async fn connected(
            &self,
            _: &ApiRequest,
            _: &RequestContext,
            _: &ViewServices,
            _: Box<dyn WebSocketSession>,
        ) -> ApiResult<()> {
            Ok(())
        }
    }

    struct ReadView;
    #[async_trait]
    impl APIView for ReadView {
        fn name(&self) -> &'static str {
            "read"
        }
        fn allowed_methods(&self) -> Vec<Method> {
            vec![Method::GET]
        }
        async fn get(
            &self,
            _: &ApiRequest,
            _: &RequestContext,
            _: &ViewServices,
        ) -> ApiResult<ApiResponse> {
            Ok(ApiResponse::empty(http::StatusCode::OK))
        }
    }

    #[test]
    fn websocket_routes_accept_http1_get_and_http2_connect() {
        let mut table = RouteTable::new();
        table
            .register(RouteSpec::websocket("/ws", "socket", Arc::new(SocketView)))
            .expect("register");
        assert!(table.resolve(&Method::GET, "/ws").is_ok());
        assert!(table.resolve(&Method::CONNECT, "/ws").is_ok());
        assert!(table.resolve(&Method::POST, "/ws").is_err());
    }

    #[test]
    fn routes_extract_safe_path_parameters() {
        let mut table = RouteTable::new();
        table
            .register(RouteSpec::http(
                "/users/{id}",
                "user-detail",
                Arc::new(ReadView),
            ))
            .expect("register");
        let route = table
            .resolve(&Method::GET, "/users/abc-123")
            .expect("resolve");
        assert_eq!(
            route.path_params.get("id").map(String::as_str),
            Some("abc-123")
        );
        assert!(table.resolve(&Method::GET, "/users/%2e%2e").is_err());
    }
}
