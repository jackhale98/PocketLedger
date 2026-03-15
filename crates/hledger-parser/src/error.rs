use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("Parse error at line {line}: {message}")]
    Syntax { line: usize, message: String },

    #[error("Invalid date: {0}")]
    InvalidDate(String),

    #[error("Invalid amount: {0}")]
    InvalidAmount(String),

    #[error("Unbalanced transaction at line {line}: {message}")]
    UnbalancedTransaction { line: usize, message: String },

    #[error("{0}")]
    Other(String),
}
