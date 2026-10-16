//! Standard locations. Environment variables override them (useful for tests
//! and portable setups): `AW_MANUAL_CONFIG_DIR`, `AW_MANUAL_DATA_DIR`.

use std::path::PathBuf;

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// `<config dir>/aw-manual/`:
/// Linux `$XDG_CONFIG_HOME/aw-manual`, macOS `~/Library/Application Support/aw-manual`,
/// Windows `%APPDATA%\aw-manual`.
pub fn config_dir() -> PathBuf {
    env_dir("AW_MANUAL_CONFIG_DIR").unwrap_or_else(|| {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("aw-manual")
    })
}

/// Local state: offline queue, persisted state and logs.
pub fn data_dir() -> PathBuf {
    env_dir("AW_MANUAL_DATA_DIR").unwrap_or_else(|| {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("aw-manual")
    })
}

pub fn database_path() -> PathBuf {
    data_dir().join("aw-manual.db")
}

pub fn log_dir() -> PathBuf {
    data_dir().join("logs")
}

/// The local host name as used for the bucket ID.
pub fn hostname() -> String {
    gethostname::gethostname()
        .to_string_lossy()
        .trim()
        .to_string()
}
