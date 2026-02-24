//! Security utilities for path validation and input sanitization.
//!
//! SEC-010: Working directory validation to prevent path traversal attacks.

use crate::{PanoptesError, Result};
use std::path::{Path, PathBuf};

/// Configuration for path security validation (SEC-010).
#[derive(Debug, Clone)]
pub struct PathSecurityConfig {
    /// Allowed base directories. Requests with working_dir outside these are rejected.
    /// Empty vec = allow all (dev mode fallback).
    pub allowed_base_dirs: Vec<PathBuf>,

    /// Default working directory when none is specified.
    pub default_working_dir: PathBuf,
}

impl Default for PathSecurityConfig {
    fn default() -> Self {
        Self {
            allowed_base_dirs: Vec::new(),
            default_working_dir: PathBuf::from("."),
        }
    }
}

/// Validate and resolve a working directory path (SEC-010).
///
/// - Returns the default if input is None or empty
/// - Rejects paths containing `..` components
/// - Canonicalizes the path if possible
/// - Checks that the path is under an allowed base directory
/// - Empty `allowed_base_dirs` = allow all (dev mode)
pub fn validate_working_dir(
    working_dir: Option<&str>,
    config: &PathSecurityConfig,
) -> Result<String> {
    // Use default if not specified
    let dir = match working_dir {
        Some(d) if !d.trim().is_empty() => d,
        _ => return Ok(config.default_working_dir.display().to_string()),
    };

    // Reject paths with .. components (path traversal)
    if dir.contains("..") {
        return Err(PanoptesError::Config(format!(
            "Working directory '{}' contains '..' path components which are not allowed",
            dir
        )));
    }

    // Attempt to canonicalize (resolve symlinks, normalize)
    let path = PathBuf::from(dir);

    // Reject dangling symbolic links (symlinks whose target doesn't exist)
    // Valid symlinks (e.g., /tmp -> /private/tmp) are allowed; canonicalize()
    // below resolves them to real paths for the allowed_base_dirs check.
    if path
        .symlink_metadata()
        .map(|m| m.is_symlink())
        .unwrap_or(false)
        && !path.exists()
    {
        return Err(PanoptesError::Config(format!(
            "Working directory '{}' is a dangling symbolic link",
            dir
        )));
    }

    let canonical = if path.exists() {
        path.canonicalize().map_err(|e| {
            PanoptesError::Config(format!(
                "Failed to resolve working directory '{}': {}",
                dir, e
            ))
        })?
    } else {
        // For non-existent paths, canonicalize the parent to resolve symlinks
        if let Some(parent) = path.parent() {
            if parent.as_os_str().is_empty() || parent == Path::new(".") {
                // Relative path with no real parent - use as-is
                path.clone()
            } else if parent.exists() {
                let canonical_parent = parent.canonicalize().map_err(|e| {
                    PanoptesError::Config(format!("Failed to resolve parent of '{}': {}", dir, e))
                })?;
                canonical_parent.join(path.file_name().unwrap_or_default())
            } else {
                return Err(PanoptesError::Config(format!(
                    "Parent directory of '{}' does not exist",
                    dir
                )));
            }
        } else {
            path.clone()
        }
    };

    // Check against allowed base directories (if configured)
    if !config.allowed_base_dirs.is_empty() {
        let is_under_allowed = config.allowed_base_dirs.iter().any(|base| {
            // Canonicalize base too for accurate comparison
            let canonical_base = if base.exists() {
                base.canonicalize().unwrap_or_else(|_| base.clone())
            } else {
                base.clone()
            };
            canonical.starts_with(&canonical_base)
        });

        if !is_under_allowed {
            return Err(PanoptesError::Config(format!(
                "Working directory '{}' is not under any allowed base directory. \
                 Allowed: {:?}",
                dir,
                config
                    .allowed_base_dirs
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
            )));
        }
    }

    Ok(canonical.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_working_dir_default() {
        let config = PathSecurityConfig::default();
        let result = validate_working_dir(None, &config).unwrap();
        assert_eq!(result, ".");
    }

    #[test]
    fn test_validate_working_dir_empty_string() {
        let config = PathSecurityConfig::default();
        let result = validate_working_dir(Some(""), &config).unwrap();
        assert_eq!(result, ".");
    }

    #[test]
    fn test_validate_working_dir_whitespace() {
        let config = PathSecurityConfig::default();
        let result = validate_working_dir(Some("  "), &config).unwrap();
        assert_eq!(result, ".");
    }

    #[test]
    fn test_validate_working_dir_rejects_traversal() {
        let config = PathSecurityConfig::default();
        assert!(validate_working_dir(Some("/home/../etc/passwd"), &config).is_err());
        assert!(validate_working_dir(Some("../../../etc"), &config).is_err());
        assert!(validate_working_dir(Some("foo/../../bar"), &config).is_err());
    }

    #[test]
    fn test_validate_working_dir_allows_valid_path() {
        let config = PathSecurityConfig::default();
        // With empty allowed_base_dirs, any non-traversal path should work
        let result = validate_working_dir(Some("/tmp"), &config);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_working_dir_enforces_allowed_base() {
        let config = PathSecurityConfig {
            allowed_base_dirs: vec![PathBuf::from("/home/user/projects")],
            default_working_dir: PathBuf::from("/home/user/projects"),
        };

        // Path not under allowed base should be rejected
        assert!(validate_working_dir(Some("/etc/passwd"), &config).is_err());
        assert!(validate_working_dir(Some("/var/log"), &config).is_err());
    }

    #[test]
    fn test_validate_working_dir_empty_allowed_is_permissive() {
        let config = PathSecurityConfig {
            allowed_base_dirs: Vec::new(),
            default_working_dir: PathBuf::from("."),
        };

        // Empty allowed_base_dirs = allow all (dev mode)
        assert!(validate_working_dir(Some("/tmp"), &config).is_ok());
        assert!(validate_working_dir(Some("/home/user"), &config).is_ok());
    }
}
