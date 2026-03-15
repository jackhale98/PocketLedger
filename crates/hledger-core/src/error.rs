use thiserror::Error;

#[derive(Debug, Error)]
pub enum LedgerError {
    #[error("Unbalanced transaction at line {line}: {message}")]
    UnbalancedTransaction { line: usize, message: String },

    #[error("Balance assertion failed at line {line}: expected {expected}, got {actual}")]
    BalanceAssertionFailed {
        line: usize,
        expected: String,
        actual: String,
    },

    #[error("Multiple postings without amounts in transaction at line {line}")]
    MultipleInferredAmounts { line: usize },

    #[error("{0}")]
    Other(String),
}
