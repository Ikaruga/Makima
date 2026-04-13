//! Configuration management

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub lm_studio: LmStudioConfig,
    pub server: ServerConfig,
    pub tools: ToolsConfig,
    pub context: ContextConfig,
}

/// LM Studio configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LmStudioConfig {
    /// API URL
    pub url: String,
    /// Model name (empty = default) — utilise pour le texte
    pub model: String,
    /// Modele vision pour OCR/PDF (utilise automatiquement par les outils qui ont besoin d'images)
    #[serde(default = "default_vision_model")]
    pub vision_model: String,
    /// Max tokens in response
    pub max_tokens: u32,
    /// Temperature for generation
    pub temperature: f32,
}

fn default_vision_model() -> String {
    "zai-org/glm-4.6v-flash".to_string()
}

/// Web server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Port to listen on
    pub port: u16,
    /// Host address
    pub host: String,
}

/// Jeu d'outils disponible
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolSet {
    /// Outils standards de Makima
    #[default]
    Standard,
    /// Outils Akari (灯) — outils enrichis style Claude Code avec web_fetch/web_search
    Akari,
}

/// Tools configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    /// Require confirmation for file writes
    pub confirm_writes: bool,
    /// Require confirmation for bash commands
    pub confirm_bash: bool,
    /// Working directory
    pub working_dir: String,
    /// Jeu d'outils (standard ou akari)
    #[serde(default)]
    pub tool_set: ToolSet,
}

/// Context configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    /// Maximum conversation history to keep
    pub max_history: usize,
    /// Enable automatic context summarization
    pub auto_summarize: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            lm_studio: LmStudioConfig {
                url: "http://localhost:1234/v1".to_string(),
                model: String::new(),
                vision_model: default_vision_model(),
                max_tokens: 4096,
                temperature: 0.7,
            },
            server: ServerConfig {
                port: 3000,
                host: "127.0.0.1".to_string(),
            },
            tools: ToolsConfig {
                confirm_writes: true,
                confirm_bash: true,
                working_dir: String::new(),
                tool_set: ToolSet::Standard,
            },
            context: ContextConfig {
                max_history: 50,
                auto_summarize: true,
            },
        }
    }
}

impl Config {
    /// Load configuration from a file
    pub async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(path).await?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load configuration from default locations
    pub async fn load_default() -> Result<Self> {
        // Try config in current directory first
        let local_config = PathBuf::from("config.toml");
        if local_config.exists() {
            return Self::load(&local_config).await;
        }

        // Try in home directory
        if let Some(home) = dirs::home_dir() {
            let home_config = home.join(".makima").join("config.toml");
            if home_config.exists() {
                return Self::load(&home_config).await;
            }
        }

        // Return default config
        Ok(Self::default())
    }

    /// Save configuration to a file
    pub async fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        fs::write(path, content).await?;
        Ok(())
    }

    /// Save to default location
    pub async fn save_default(&self) -> Result<PathBuf> {
        let config_dir = dirs::home_dir()
            .map(|h| h.join(".makima"))
            .unwrap_or_else(|| PathBuf::from("."));

        fs::create_dir_all(&config_dir).await?;

        let config_path = config_dir.join("config.toml");
        self.save(&config_path).await?;

        Ok(config_path)
    }
}
