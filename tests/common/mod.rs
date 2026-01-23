//! Common test infrastructure for Wraith compiler tests
//!
//! This module provides shared utilities, helpers, and fixtures
//! used across the test suite.

pub mod assertions;
pub mod fixtures;
pub mod harness;

// Re-export commonly used items
pub use assertions::*;
pub use harness::*;
