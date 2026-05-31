//! OS-native secret storage for the API key.
//!
//! The key never lives in the SQLite settings blob: `save_settings` strips it
//! out and stashes it here (Windows Credential Manager via the `keyring`
//! crate), and the command layer re-injects it just before building an
//! [`crate::provider::ApiProvider`]. The UI therefore sees an empty field after
//! reload — the key is write-only from the frontend's point of view.

const SERVICE: &str = "KaigiAI";
const ACCOUNT: &str = "apiKey";

/// Persist the API key in the OS keychain. Best-effort: a failure (e.g. no
/// secret backend available) is logged but not fatal — the app keeps working,
/// just without a stored key.
pub fn set_api_key(key: &str) {
    match keyring::Entry::new(SERVICE, ACCOUNT) {
        Ok(entry) => {
            if let Err(e) = entry.set_password(key) {
                log::error!("failed to store API key in keychain: {e}");
            }
        }
        Err(e) => log::error!("failed to open keychain entry: {e}"),
    }
}

/// Read the API key from the OS keychain, or `None` if unset/unavailable.
pub fn get_api_key() -> Option<String> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .ok()?
        .get_password()
        .ok()
}
