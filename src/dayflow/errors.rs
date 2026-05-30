//! Dayflow error type.
//!
//! `DayflowError` is defined in the contracts layer (`contracts::errors`) so the
//! error taxonomy stays centralized and `GentleEyeError` can wrap it without a
//! layering inversion. This module re-exports it for ergonomic `dayflow::errors`
//! / `dayflow::DayflowError` access.

pub use crate::contracts::errors::DayflowError;

/// Convenience result alias for dayflow operations.
pub type DayflowResult<T> = std::result::Result<T, DayflowError>;
