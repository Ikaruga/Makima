//! CLI interface module

pub mod confirm;
pub mod repl;
pub mod tool_events;
pub mod tool_prompts;
pub mod ui;

pub use repl::Repl;
pub use tool_events::{tool_event_channel, ToolEvent, ToolEventReceiver, ToolEventSender};
pub use tool_prompts::{
    tool_clear, tool_done, tool_progress, tool_prompt_choice, tool_prompt_line, tool_prompt_text,
    tool_prompt_yesno, tool_status,
};
pub use ui::{CliUI, TokenStats};

/// Execution mode for the assistant
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionMode {
    /// Plan mode: exploration only, tools are NOT executed
    Plan,
    /// Edit mode: tools can be executed normally
    #[default]
    Edit,
}

impl ExecutionMode {
    /// Toggle between Plan and Edit modes
    pub fn toggle(&self) -> Self {
        match self {
            ExecutionMode::Plan => ExecutionMode::Edit,
            ExecutionMode::Edit => ExecutionMode::Plan,
        }
    }

    /// Get the display name for the mode
    pub fn display_name(&self) -> &'static str {
        match self {
            ExecutionMode::Plan => "MODE PLAN",
            ExecutionMode::Edit => "MODE EDIT",
        }
    }
}
