//! Protocol messages for CLI-daemon communication.

mod events;
mod messages;

pub use events::{LogEvent, Outcome, TestEvent};
pub use messages::{ErrorCode, Request, Response};
