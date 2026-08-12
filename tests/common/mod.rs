//! Common test infrastructure for Wraith compiler tests
//!
//! This module provides shared utilities and helpers
//! used across the test suite.

pub mod assertions;
pub mod devices;
pub mod exec;
pub mod harness;

// Re-export commonly used items
// Different test binaries pull in different halves of this module; a binary
// that uses none of the assertion helpers should not warn about them.
#[allow(unused_imports)]
pub use assertions::*;
pub use harness::*;
