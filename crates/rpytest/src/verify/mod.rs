//! Drop-in compatibility verification.
//!
//! Compares rpytest output against pytest to ensure compatibility.

mod diff;
mod dropin;

#[allow(unused_imports)]
pub use diff::{DiffKind, OutputDiff};
#[allow(unused_imports)]
pub use dropin::{verify_dropin, VerifyConfig, VerifyResult};
