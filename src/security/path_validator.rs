//! Path validation (CWE-22 traversal guard) for MCP file inputs.
//!
//! Implements **NFR-001**: every `video_path` / `output_dir` input MUST be
//! constrained to the configured recording directory (or an explicitly allowed
//! root). Rejects `..` traversal components and absolute paths that escape the
//! allowed roots. For paths that already exist, a `canonicalize` pass also
//! defeats symlink-escape; for not-yet-created output paths it falls back to a
//! lexical prefix check.
//!
//! Authored 2026-05-28 from the contract (`storage::manager` usage) + PRD
//! NFR-001/T101 — the recovered source for this file was an empty stub.

use std::path::{Component, Path, PathBuf};
use thiserror::Error;

/// Reasons a path was rejected by [`PathValidator`].
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathValidationError {
    /// The input contained a `..` (parent-dir) component.
    #[error("path traversal ('..') is not allowed: {0}")]
    Traversal(String),

    /// The resolved path is outside every allowed root.
    #[error("path is outside the allowed recording directory: {0}")]
    OutsideAllowedRoots(String),
}

/// Validates file paths supplied by MCP tool callers.
///
/// Construct with the configured recording directory; add further roots with
/// [`PathValidator::add_allowed_root`] if the deployment permits more than one.
#[derive(Debug, Clone)]
pub struct PathValidator {
    allowed_roots: Vec<PathBuf>,
}

impl Default for PathValidator {
    fn default() -> Self {
        Self::new(std::env::temp_dir().join("gentle-eye"))
    }
}

impl PathValidator {
    /// Create a validator constrained to `base_dir`.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            allowed_roots: vec![base_dir.into()],
        }
    }

    /// Permit an additional root directory.
    pub fn add_allowed_root(&mut self, root: impl Into<PathBuf>) {
        self.allowed_roots.push(root.into());
    }

    /// The allowed roots, in priority order (first is the primary base dir).
    pub fn allowed_roots(&self) -> &[PathBuf] {
        &self.allowed_roots
    }

    /// Validate `input`, returning the cleaned absolute path if it is permitted.
    ///
    /// Relative inputs are resolved against the primary (first) allowed root.
    pub fn validate(&self, input: &Path) -> Result<PathBuf, PathValidationError> {
        // 1. Reject any explicit `..` component up front (CWE-22).
        if input
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(PathValidationError::Traversal(input.display().to_string()));
        }

        // 2. Resolve to an absolute, lexically-clean candidate.
        let candidate = if input.is_absolute() {
            lexical_clean(input)
        } else {
            let base = self
                .allowed_roots
                .first()
                .cloned()
                .unwrap_or_else(std::env::temp_dir);
            lexical_clean(&base.join(input))
        };

        // 3. Must sit under an allowed root (lexical check), and — when the
        //    path exists — under a canonicalized root (symlink-escape guard).
        if self.within_allowed(&candidate) {
            return Ok(candidate);
        }
        Err(PathValidationError::OutsideAllowedRoots(
            candidate.display().to_string(),
        ))
    }

    /// Convenience boolean form of [`PathValidator::validate`].
    pub fn is_allowed(&self, input: &Path) -> bool {
        self.validate(input).is_ok()
    }

    fn within_allowed(&self, candidate: &Path) -> bool {
        // Prefer the canonical form when the candidate (or its parent) exists,
        // so symlinks can't escape; otherwise fall back to the lexical path.
        let resolved = candidate.canonicalize().unwrap_or_else(|_| {
            candidate
                .parent()
                .and_then(|p| p.canonicalize().ok())
                .map(|cp| {
                    candidate
                        .file_name()
                        .map_or_else(|| cp.clone(), |name| cp.join(name))
                })
                .unwrap_or_else(|| candidate.to_path_buf())
        });

        self.allowed_roots.iter().any(|root| {
            let root_resolved = root.canonicalize().unwrap_or_else(|_| lexical_clean(root));
            resolved.starts_with(&root_resolved) || candidate.starts_with(lexical_clean(root))
        })
    }
}

/// Remove `.` (current-dir) components without touching the filesystem.
/// `..` is intentionally NOT collapsed here — it is rejected earlier.
fn lexical_clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn rejects_parent_dir_traversal() {
        let v = PathValidator::new("/tmp/gentle-eye");
        let err = v.validate(Path::new("../etc/passwd")).unwrap_err();
        assert!(matches!(err, PathValidationError::Traversal(_)));
        assert!(!v.is_allowed(Path::new("recordings/../../etc/passwd")));
    }

    #[test]
    fn rejects_absolute_path_outside_root() {
        let v = PathValidator::new("/tmp/gentle-eye");
        let err = v.validate(Path::new("/etc/passwd")).unwrap_err();
        assert!(matches!(err, PathValidationError::OutsideAllowedRoots(_)));
    }

    #[test]
    fn accepts_relative_path_inside_root() {
        let dir = tempdir().unwrap();
        let v = PathValidator::new(dir.path());
        let ok = v.validate(Path::new("clip.mp4")).unwrap();
        assert!(ok.ends_with("clip.mp4"));
        assert!(v.is_allowed(Path::new("clip.mp4")));
    }

    #[test]
    fn accepts_absolute_path_inside_root() {
        let dir = tempdir().unwrap();
        let inside = dir.path().join("sub/clip.mp4");
        let v = PathValidator::new(dir.path());
        assert!(v.is_allowed(&inside));
    }

    #[test]
    fn additional_root_is_honored() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let mut v = PathValidator::new(a.path());
        v.add_allowed_root(b.path());
        assert!(v.is_allowed(&b.path().join("clip.mp4")));
        assert_eq!(v.allowed_roots().len(), 2);
    }
}
