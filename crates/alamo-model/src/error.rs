//! Error type for model and animation parsing.

use std::fmt;

/// Parse errors. Malformed input never panics; it produces one of these.
#[derive(Debug)]
pub enum Error {
    /// Data ended before a read completed.
    Truncated(&'static str),
    /// Structurally invalid data.
    Invalid(String),
    /// A valid .alo file that is not a model (e.g. a particle system).
    NotAModel(&'static str),
    /// I/O error while reading from a file source.
    Io(std::io::Error),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated(what) => write!(f, "truncated data ({what})"),
            Error::Invalid(msg) => write!(f, "invalid data: {msg}"),
            Error::NotAModel(what) => write!(f, "not a model: {what}"),
            Error::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
