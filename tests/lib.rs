//! Consolidated test suite for the Wraith compiler
//!
//! Test Organization:
//! - common/     - Shared test infrastructure
//! - errors/     - Error message tests (by phase)
//! - integration/ - Individual compiler phase tests
//! - e2e/        - End-to-end feature tests

#[path = "common/mod.rs"]
mod common;

mod errors;
mod integration;
mod e2e;
