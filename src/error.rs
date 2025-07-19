use std::fmt;
use std::error::Error as StdError;

#[derive(Debug)]
pub enum ExchangeError {
    ConfigError(String),
    NetworkError(String),
    ParseError(String),
    ValidationError(String),
    OrderError(String),
    PersistenceError(String),
}

impl fmt::Display for ExchangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExchangeError::ConfigError(msg) => write!(f, "Configuration error: {}", msg),
            ExchangeError::NetworkError(msg) => write!(f, "Network error: {}", msg),
            ExchangeError::ParseError(msg) => write!(f, "Parse error: {}", msg),
            ExchangeError::ValidationError(msg) => write!(f, "Validation error: {}", msg),
            ExchangeError::OrderError(msg) => write!(f, "Order error: {}", msg),
            ExchangeError::PersistenceError(msg) => write!(f, "Persistence error: {}", msg),
        }
    }
}

impl StdError for ExchangeError {}

impl From<std::io::Error> for ExchangeError {
    fn from(err: std::io::Error) -> Self {
        ExchangeError::PersistenceError(err.to_string())
    }
}

impl From<csv::Error> for ExchangeError {
    fn from(err: csv::Error) -> Self {
        ExchangeError::ParseError(err.to_string())
    }
}

impl From<reqwest::Error> for ExchangeError {
    fn from(err: reqwest::Error) -> Self {
        ExchangeError::NetworkError(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ExchangeError>;
