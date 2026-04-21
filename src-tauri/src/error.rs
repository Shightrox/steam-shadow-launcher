use serde::Serialize;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("IO error: {0}")]
    Io(String),
    #[error("Registry error: {0}")]
    Registry(String),
    #[error("Workspace invalid: {0}")]
    Workspace(String),
    #[error("Junction error: {0}")]
    Junction(String),
    #[error("Account not ready: {0}")]
    NotReady(String),
    #[error("Process error: {0}")]
    Process(String),
    #[error("Config error: {0}")]
    Config(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Other: {0}")]
    Other(String),
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Config(e.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
