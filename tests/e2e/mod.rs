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
mod enum_values;
mod enums;
mod execution;
mod frames;
mod functions;
mod import_diagnostics;
mod imports;
mod inline;
mod interrupts;
mod interrupts_exec;
mod language_features;
mod local_arrays;
mod loop_sweep;
mod math16;
mod memory;
mod operators;
mod org_conflicts;
mod placement;
mod pointer_escape;
mod pointer_syntax;
mod pointers;
mod statics;
mod stdlib;
mod strings_slices;
mod types;
mod variables;
mod vtable;
