//! Errors carry the text Python's exception would print, and the kind the
//! Python code catches them by.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// ValueError and its subclasses (JSONDecodeError, UnicodeDecodeError).
    Value(String),
    /// OSError and its subclasses.
    Os(String),
    /// CatalogueBusyError.
    Busy(String),
    /// zipfile.BadZipFile.
    BadZip(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Value(m) | Error::Os(m) | Error::Busy(m) | Error::BadZip(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub fn value<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Value(message.into()))
}
