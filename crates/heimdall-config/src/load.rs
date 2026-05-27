use crate::error::ConfigError;
use crate::schema::ConfigFile;
use std::fs;
use std::path::{Path, PathBuf};

/// Read and parse a `heimdall.toml`. Does NOT validate cross-references.
/// Call `crate::validate::validate(&cfg)` for that.
pub fn load_from_path(path: impl AsRef<Path>) -> Result<ConfigFile, ConfigError> {
    let path = PathBuf::from(path.as_ref());
    let raw = fs::read_to_string(&path).map_err(|source| ConfigError::Io {
        path: path.clone(),
        source,
    })?;
    let cfg: ConfigFile = toml::from_str(&raw).map_err(|source| ConfigError::Parse {
        path: path.clone(),
        source,
    })?;
    tracing::debug!(?path, "loaded heimdall config");
    Ok(cfg)
}
