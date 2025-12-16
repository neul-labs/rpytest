//! Drop-in compatibility verification.
//!
//! Compares rpytest output against pytest to ensure compatibility.

mod dropin;
mod diff;

pub use dropin::{verify_dropin, VerifyResult, VerifyConfig};
pub use diff::{OutputDiff, DiffKind};
