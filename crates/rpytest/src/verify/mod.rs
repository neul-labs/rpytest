//! Drop-in compatibility verification.
//!
//! Compares rpytest output against pytest to ensure compatibility.

mod diff;
mod dropin;

pub use diff::{DiffKind, OutputDiff};
pub use dropin::{verify_dropin, VerifyConfig, VerifyResult};
