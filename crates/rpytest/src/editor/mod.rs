//! Editor protocol for IDE integration.
//!
//! Provides JSON-RPC over stdio or TCP for editor plugins to:
//! - Run the nearest test to cursor
//! - List tests in a file
//! - Get test status/results

mod protocol;
mod server;

#[allow(unused_imports)]
pub use protocol::{EditorRequest, EditorResponse, TestLocation};
#[allow(unused_imports)]
pub use server::EditorServer;
