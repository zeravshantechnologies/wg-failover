use thiserror::Error;

/// Errors that can occur in the WireGuard failover system
#[derive(Error, Debug)]
pub enum FailoverError {
    /// IO error
    #[error("IO error: {0}")]
    IOError(#[from] std::io::Error),

    /// Unknown error
    #[error("Unknown error: {0}")]
    Unknown(String),
}



/// Convert anyhow errors to FailoverError
impl From<anyhow::Error> for FailoverError {
    fn from(err: anyhow::Error) -> Self {
        FailoverError::Unknown(err.to_string())
    }
}