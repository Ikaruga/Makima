//! Event system for tool interactions with the UI
//!
//! Tools can send events to update the tool zone in the fixed panel
//! without interfering with the main LLM response streaming.

use tokio::sync::mpsc;

/// Events emitted by tools for UI display
#[derive(Debug, Clone)]
pub enum ToolEvent {
    /// Display a status message in line 1 of the tool zone
    Status(String),
    /// Display a prompt/options in line 2 of the tool zone
    Prompt(String),
    /// Display both lines at once (more efficient)
    Both { line1: String, line2: String },
    /// Clear the tool zone
    Clear,
}

/// Sender for tools to emit UI events
pub type ToolEventSender = mpsc::UnboundedSender<ToolEvent>;

/// Receiver for the REPL to handle UI events
pub type ToolEventReceiver = mpsc::UnboundedReceiver<ToolEvent>;

/// Create a new event channel for tool UI communication
pub fn tool_event_channel() -> (ToolEventSender, ToolEventReceiver) {
    mpsc::unbounded_channel()
}
