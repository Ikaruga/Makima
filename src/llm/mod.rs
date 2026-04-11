//! LLM client module for LM Studio integration

pub mod client;
pub mod streaming;
pub mod tool_parser;
pub mod types;

pub use client::{LmStudioClient, StreamEvent};
pub use streaming::{StreamAccumulator, StreamConsumer};
pub use tool_parser::{generate_tool_prompt, generate_akari_prompt, ToolParser};
pub use types::*;
