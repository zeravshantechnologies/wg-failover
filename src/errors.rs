use thiserror::Error;

/// Errors that can occur in the WireGuard failover system
#[derive(Error, Debug)]
pub enum FailoverError {
    /// Interface could not be found
    #[error("Interface not found: {0}")]
    InterfaceNotFound(String),

    /// Gateway could not be found for the specified interface
    #[error("Gateway not found for interface: {0}")]
    GatewayNotFound(String),

    /// Command execution failed
    #[error("Failed to execute command: {0}")]
    CommandExecution(String),

    /// Route modification failed
    #[error("Route modification failed: {0}")]
    RouteModificationFailed(String),

    /// WireGuard interface restart failed
    #[error("WireGuard interface restart failed: {0}")]
    WireGuardRestartFailed(String),

    /// Network connectivity check failed
    #[error("Network connectivity check failed: {0}")]
    ConnectivityCheckFailed(String),

    /// Configuration is invalid
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    /// Operating system is not supported
    #[error("Unsupported operating system")]
    UnsupportedOS,

    /// Insufficient permissions
    #[error("Insufficient permissions (try running as root)")]
    InsufficientPermissions,

    /// IO error
    #[error("IO error: {0}")]
    IOError(#[from] std::io::Error),

    /// Unknown error
    #[error("Unknown error: {0}")]
    Unknown(String),
}

/// Shorthand result type for failover operations
pub type FailoverResult<T> = Result<T, FailoverError>;

/// Convert anyhow errors to FailoverError
impl From<anyhow::Error> for FailoverError {
    fn from(err: anyhow::Error) -> Self {
        FailoverError::Unknown(err.to_string())
    }
}