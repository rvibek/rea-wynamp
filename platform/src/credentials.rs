//! Secret storage (passwords, API tokens).
//!
//! Secrets never live in the library database or settings files — they go to
//! the OS credential vault: macOS Keychain, Windows Credential Manager,
//! Linux Secret Service (via the `keyring` crate, v0.5.0).

use rea_wynamp_core::{Error, Result};

/// Stores secrets in the OS credential vault.
///
/// `service` namespaces secrets per backend (e.g. `"navidrome"`), `account`
/// identifies the entry (e.g. the configured server id).
pub trait Credentials: Send + Sync {
    /// Fetch a stored secret; `Ok(None)` when absent.
    fn get(&self, service: &str, account: &str) -> Result<Option<String>>;

    /// Store (or overwrite) a secret.
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<()>;

    /// Delete a secret; deleting an absent entry is not an error.
    fn delete(&self, service: &str, account: &str) -> Result<()>;
}

/// Placeholder used until the keyring-backed implementation lands in v0.5.0.
///
/// Reads always succeed with "nothing stored"; writes fail loudly with
/// [`Error::Unsupported`] so missing integration can't be mistaken for an
/// empty vault.
#[derive(Debug, Default, Clone, Copy)]
pub struct StubCredentials;

const UNSUPPORTED: &str = "secure credential storage arrives in v0.5.0";

impl Credentials for StubCredentials {
    fn get(&self, _service: &str, _account: &str) -> Result<Option<String>> {
        Ok(None)
    }

    fn set(&self, _service: &str, _account: &str, _secret: &str) -> Result<()> {
        Err(Error::Unsupported(UNSUPPORTED.to_string()))
    }

    fn delete(&self, _service: &str, _account: &str) -> Result<()> {
        Err(Error::Unsupported(UNSUPPORTED.to_string()))
    }
}
