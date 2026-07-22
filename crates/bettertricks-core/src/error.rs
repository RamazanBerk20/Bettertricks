use thiserror::Error;

pub type Result<T> = std::result::Result<T, BettertricksError>;

#[derive(Debug, Error)]
pub enum BettertricksError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("invalid recipe: {0}")]
    Recipe(String),
    #[error("catalog error: {0}")]
    Catalog(String),
    #[error("prefix not found: {0}")]
    PrefixNotFound(String),
    #[error("recipe not found: {0}")]
    RecipeNotFound(String),
    #[error("operation not found: {0}")]
    OperationNotFound(String),
    #[error("operation conflict: {0}")]
    Conflict(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error("checksum mismatch for {file}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        file: String,
        expected: String,
        actual: String,
    },
    #[error("external command failed: {program} exited with {code:?}")]
    CommandFailed { program: String, code: Option<i32> },
    #[error("external command failed: {program} {status}. {detail}")]
    CommandFailedWithOutput {
        program: String,
        status: String,
        detail: String,
    },
    #[error("unsupported operation: {0}")]
    Unsupported(String),
    #[error("security policy rejected this operation: {0}")]
    Security(String),
    #[error("serialization error: {0}")]
    Serialization(String),
}

impl From<toml::de::Error> for BettertricksError {
    fn from(value: toml::de::Error) -> Self {
        Self::Serialization(value.to_string())
    }
}

impl From<serde_json::Error> for BettertricksError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value.to_string())
    }
}

impl From<reqwest::Error> for BettertricksError {
    fn from(value: reqwest::Error) -> Self {
        Self::Io(std::io::Error::other(value))
    }
}
