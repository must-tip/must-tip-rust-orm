use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestAction {
    List,
    Retrieve,
    Create,
    Update,
    PartialUpdate,
    Destroy,
}

/// Application-supplied REST authorization boundary.
///
/// Implementations are expected to hold an already authenticated, authoritative caller context.
/// Write and instance checks deny by default so an incomplete policy fails closed.
pub trait RestAccessPolicy<T: Entity>: Send + Sync {
    fn scope_predicate(&self, _action: RestAction) -> Result<Option<Predicate>, RestError> {
        Ok(None)
    }

    fn authorize_write(
        &self,
        _action: RestAction,
        _fields: &Map<String, Value>,
    ) -> Result<(), RestError> {
        Err(RestError::new(
            "permission_denied",
            "write access was not authorized",
        ))
    }

    fn authorize_instance(&self, _action: RestAction, _instance: &T) -> Result<(), RestError> {
        Err(RestError::new(
            "permission_denied",
            "object access was not authorized",
        ))
    }
}
