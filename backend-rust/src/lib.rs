//! Library target.
//!
//! The crate was binary-only, which meant nothing outside `main.rs` could be
//! tested — and until 2026-08-05 nothing was. Two bugs regressed within a
//! single session because of it: the state-save race was "fixed" with a
//! sequence guard that did not work and had to be fixed again, and the profit
//! ledger was corrupted twice in different ways.
//!
//! Exposing the modules here lets `tests/` exercise the accounting logic
//! directly. `main.rs` keeps its own `mod` declarations and is unaffected.

pub mod config;
pub mod models;
pub mod services;
// `services::agentic_test` refers to `crate::state::AppState`, so the module
// graph pulls this in whether or not a test touches it directly.
pub mod state;
