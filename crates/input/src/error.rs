// SPDX-License-Identifier: GPL-3.0-or-later

//! The error returned when a configuration is refused.

use std::fmt;

/// A configuration that the crate refuses to use.
///
/// The message says which rule was broken. The type carries no recoverable
/// state: a refused configuration is either edited by the caller or replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    message: String,
}

impl ConfigError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The reason the configuration was refused.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConfigError {}
