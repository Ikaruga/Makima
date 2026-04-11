//! Project context and metadata

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

/// Project context information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectContext {
    /// Root directory of the project
    pub root: PathBuf,
    /// Project name (from directory name or config)
    pub name: String,
    /// Detected project type
    pub project_type: Option<ProjectType>,
    /// Key files in the project
    pub key_files: Vec<String>,
    /// Project description (if available)
    pub description: Option<String>,
}

/// Types of projects we can detect
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectType {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
    Java,
    CSharp,
    Ruby,
    Php,
    Unknown,
}

impl ProjectType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProjectType::Rust => "Rust",
            ProjectType::Python => "Python",
            ProjectType::JavaScript => "JavaScript",
            ProjectType::TypeScript => "TypeScript",
            ProjectType::Go => "Go",
            ProjectType::Java => "Java",
            ProjectType::CSharp => "C#",
            ProjectType::Ruby => "Ruby",
            ProjectType::Php => "PHP",
            ProjectType::Unknown => "Unknown",
        }
    }
}

impl ProjectContext {
    /// Create a new project context from a directory
    pub async fn from_directory(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = path.as_ref().canonicalize().unwrap_or_else(|_| path.as_ref().to_path_buf());
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "project".to_string());

        let mut context = Self {
            root: root.clone(),
            name,
            project_type: None,
            key_files: Vec::new(),
            description: None,
        };

        // Detect project type and key files
        context.detect_project_type().await;

        Ok(context)
    }

    /// Detect the project type based on marker files
    async fn detect_project_type(&mut self) {
        let markers = [
            ("Cargo.toml", ProjectType::Rust),
            ("pyproject.toml", ProjectType::Python),
            ("requirements.txt", ProjectType::Python),
            ("setup.py", ProjectType::Python),
            ("package.json", ProjectType::JavaScript),
            ("tsconfig.json", ProjectType::TypeScript),
            ("go.mod", ProjectType::Go),
            ("pom.xml", ProjectType::Java),
            ("build.gradle", ProjectType::Java),
            ("*.csproj", ProjectType::CSharp),
            ("Gemfile", ProjectType::Ruby),
            ("composer.json", ProjectType::Php),
        ];

        for (marker, project_type) in markers {
            let marker_path = self.root.join(marker);
            if marker_path.exists() {
                self.project_type = Some(project_type);
                self.key_files.push(marker.to_string());
                break;
            }
        }

        // Add common important files
        let common_files = ["README.md", "README.rst", "LICENSE", ".gitignore"];
        for file in common_files {
            if self.root.join(file).exists() {
                self.key_files.push(file.to_string());
            }
        }

        // Try to read description from README
        if let Some(readme) = self.key_files.iter().find(|f| f.to_lowercase().starts_with("readme")) {
            if let Ok(content) = fs::read_to_string(self.root.join(readme)).await {
                // Take first paragraph as description
                self.description = content
                    .lines()
                    .skip_while(|l| l.starts_with('#') || l.trim().is_empty())
                    .take_while(|l| !l.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join(" ")
                    .chars()
                    .take(200)
                    .collect::<String>()
                    .into();
            }
        }

        if self.project_type.is_none() {
            self.project_type = Some(ProjectType::Unknown);
        }
    }

    /// Generate a context summary for the LLM
    pub fn to_summary(&self) -> String {
        let mut parts = vec![
            format!("Project: {}", self.name),
            format!("Location: {}", self.root.display()),
        ];

        if let Some(ref pt) = self.project_type {
            parts.push(format!("Type: {}", pt.as_str()));
        }

        if !self.key_files.is_empty() {
            parts.push(format!("Key files: {}", self.key_files.join(", ")));
        }

        if let Some(ref desc) = self.description {
            parts.push(format!("Description: {}", desc));
        }

        parts.join("\n")
    }
}
