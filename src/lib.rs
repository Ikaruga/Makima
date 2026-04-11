//! Makima - A local coding assistant powered by LM Studio
//!
//! This library provides the core functionality for the Makima CLI and web interface.

pub mod cli;
pub mod config;
pub mod context;
pub mod llm;
pub mod tools;
pub mod web;

pub use config::Config;
pub use context::{Conversation, ProjectContext};
pub use llm::LmStudioClient;
pub use tools::{ToolExecutor, ToolRegistry};
