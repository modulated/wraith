//! End-to-end feature tests
//!
//! Tests complete language features from source to assembly output

mod bcd;
mod bcd_validation;
mod complex_features;
mod control_flow;
mod cpu_flags;
mod dead_code;
mod devices;
mod enums;
mod execution;
mod frames;
mod functions;
mod imports;
mod inline;
mod interrupts;
mod interrupts_exec;
mod language_features;
mod loop_sweep;
mod math16;
mod memory;
mod operators;
mod placement;
mod org_conflicts;
mod statics;
mod stdlib;
mod strings_slices;
mod types;
mod variables;
mod vtable;
