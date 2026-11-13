#[derive(Debug)]
pub(crate) enum TransportError {
    /// Request timed out
    Timeout,
    /// Unrecoverable failure. Future requests will fail.
    Unrecoverable,
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "request timed out"),
            Self::Unrecoverable => write!(f, "unrecoverable mcp failure, future requests will fail"),
        }
    }
}

impl std::error::Error for TransportError {}
