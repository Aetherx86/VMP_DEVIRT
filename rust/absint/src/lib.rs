//! An abstract interpretation static analyzer.
//!
//! It proves facts about programs without running them. It computes an over-approximation of every
//! state a program can reach, so a fault that cannot occur in that over-approximation cannot occur at
//! all. The reverse does not hold: "possible" can be a false alarm.
//!
//! - [`analysis`] proves the absence of out of bounds accesses, null dereferences, division by zero,
//!   use of uninitialized variables, and assertion failures, using interval and nullness domains
//! - [`races`] proves the absence of data races on globals, through locksets
//! - [`wasm`] applies this to WebAssembly, to prove memory accesses in bounds
//! - [`x86`] applies a stack-specific version to x86-64, to prove a function cannot overwrite its return
//!   address through its own stack pointer
//!
//! The intervals are non-relational: `i < n` for a variable `n` proves nothing about `i` unless `n` has
//! a known range. Each frontend documents what it assumes.

pub mod analysis;
pub mod domain;
pub mod ir;
pub mod races;
pub mod text;
pub mod wasm;
pub mod x86;
