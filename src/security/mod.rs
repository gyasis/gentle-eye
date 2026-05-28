//! Security: path validation, rate limiting, and UUID validation for MCP inputs.
//!
//! Skeleton (Wave 0, 2026-05-28): module declarations only. The submodules are
//! authored in later waves; re-exports are added there to avoid unused-import
//! errors under `-D warnings`.

pub mod path_validator;
pub mod rate_limiter;
pub mod uuid_validator;

pub use path_validator::{PathValidationError, PathValidator};
pub use rate_limiter::{RateLimitExceeded, RateLimiter};
