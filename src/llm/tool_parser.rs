//! Parser for tool calls with multi-format support
//! Mode 1: Parse native OpenAI-format tool_calls
//! Mode 2: Parse <tool> XML tags from text (fallback for models without native support)
//! Mode 3: Parse emoji format (🔧 tool_name> {...})
//! Mode 4: Parse markdown code block format (```tool:tool_name {...} ```)
//! Mode 5: Parse function call format (tool_name({...}))

use crate::llm::types::{ParsedToolCall, ToolCall};
use regex::Regex;
use std::collections::HashMap;
use uuid::Uuid;

/// Parse tool calls from the assistant's response
pub struct ToolParser {
    /// Regex for matching <tool> tags
    tool_tag_regex: Regex,
    /// Regex for matching key="value" or key='value' attributes
    attr_regex: Regex,
    /// Regex for matching XML-style tags like <path>value</path>
    xml_tag_regex: Regex,
    /// Regex for matching emoji format: 🔧 tool_name> {...} or 🔧 tool_name {...}
    emoji_tool_regex: Regex,
    /// Regex for matching markdown code block: ```tool:tool_name\n{...}\n```
    markdown_tool_regex: Regex,
    /// Regex for matching function call format: tool_name({...})
    function_call_regex: Regex,
}

impl Default for ToolParser {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolParser {
    pub fn new() -> Self {
        Self {
            // Match <tool name="...">...</tool> or <tool name="..." />
            tool_tag_regex: Regex::new(
                r#"<tool\s+name\s*=\s*["']([^"']+)["']\s*>([\s\S]*?)</tool>|<tool\s+name\s*=\s*["']([^"']+)["']\s*/>"#
            ).expect("Invalid tool_tag_regex pattern"),
            // Match key="value" or key='value' pairs
            attr_regex: Regex::new(r#"(\w+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#)
                .expect("Invalid attr_regex pattern"),
            // Match XML-style tags like <path>value</path>
            xml_tag_regex: Regex::new(r#"<(\w+)>([\s\S]*?)</(\w+)>"#)
                .expect("Invalid xml_tag_regex pattern"),
            // Match emoji format: 🔧 tool_name> {...} or 🔧 tool_name {...}
            emoji_tool_regex: Regex::new(
                r#"🔧\s*(\w+)>?\s*(\{[\s\S]*?\})"#
            ).expect("Invalid emoji_tool_regex pattern"),
            // Match markdown code block: ```tool:tool_name\n{...}\n```
            markdown_tool_regex: Regex::new(
                r#"```tool:(\w+)\s*\n([\s\S]*?)\n```"#
            ).expect("Invalid markdown_tool_regex pattern"),
            // Match function call format: tool_name({...})
            function_call_regex: Regex::new(
                r#"(\w+)\s*\(\s*(\{[\s\S]*?\})\s*\)"#
            ).expect("Invalid function_call_regex pattern"),
        }
    }

    /// Parse tool calls from native API format
    pub fn parse_native(&self, tool_calls: &[ToolCall]) -> Vec<ParsedToolCall> {
        tool_calls
            .iter()
            .filter_map(|tc| {
                let arguments: HashMap<String, serde_json::Value> =
                    serde_json::from_str(&tc.function.arguments).ok()?;
                Some(ParsedToolCall::new(
                    tc.id.clone(),
                    tc.function.name.clone(),
                    arguments,
                ))
            })
            .collect()
    }

    /// Parse tool calls from text content (fallback mode)
    /// Supports multiple formats:
    /// 1. XML: <tool name="read_file">{"path": "..."}</tool>
    /// 2. Emoji: 🔧 read_file> {"path": "..."}
    /// 3. Markdown: ```tool:read_file\n{"path": "..."}\n```
    /// 4. Function: read_file({"path": "..."})
    pub fn parse_from_text(&self, text: &str) -> Vec<ParsedToolCall> {
        // Try XML format first (preferred)
        let results = self.parse_xml_format(text);
        if !results.is_empty() {
            return results;
        }

        // Try emoji format (🔧 tool_name> {...})
        let results = self.parse_emoji_format(text);
        if !results.is_empty() {
            return results;
        }

        // Try markdown code block format (```tool:tool_name {...} ```)
        let results = self.parse_markdown_format(text);
        if !results.is_empty() {
            return results;
        }

        // Try function call format (tool_name({...}))
        let results = self.parse_function_format(text);
        if !results.is_empty() {
            return results;
        }

        Vec::new()
    }

    /// Parse XML format: <tool name="...">...</tool>
    fn parse_xml_format(&self, text: &str) -> Vec<ParsedToolCall> {
        let mut results = Vec::new();

        for cap in self.tool_tag_regex.captures_iter(text) {
            // Get tool name (from either capture group 1 or 3)
            let name = cap.get(1).or_else(|| cap.get(3)).map(|m| m.as_str().to_string());

            // Get content (from capture group 2, if present)
            let content = cap.get(2).map(|m| m.as_str().trim().to_string());

            if let Some(name) = name {
                let arguments = if let Some(content) = content {
                    self.parse_tool_content(&content)
                } else {
                    HashMap::new()
                };

                results.push(ParsedToolCall::new(
                    Uuid::new_v4().to_string(),
                    name,
                    arguments,
                ));
            }
        }

        results
    }

    /// Parse emoji format: 🔧 tool_name> {...} or 🔧 tool_name {...}
    fn parse_emoji_format(&self, text: &str) -> Vec<ParsedToolCall> {
        let mut results = Vec::new();

        for cap in self.emoji_tool_regex.captures_iter(text) {
            if let (Some(name), Some(json_content)) = (cap.get(1), cap.get(2)) {
                let name = name.as_str().to_string();
                let content = json_content.as_str().trim();

                // Try parsing as-is first, then with fixed Windows paths
                let arguments = serde_json::from_str::<HashMap<String, serde_json::Value>>(content)
                    .or_else(|_| {
                        let fixed = Self::fix_windows_paths_in_json(content);
                        serde_json::from_str::<HashMap<String, serde_json::Value>>(&fixed)
                    });

                if let Ok(arguments) = arguments {
                    results.push(ParsedToolCall::new(
                        Uuid::new_v4().to_string(),
                        name,
                        arguments,
                    ));
                }
            }
        }

        results
    }

    /// Parse markdown format: ```tool:tool_name\n{...}\n```
    fn parse_markdown_format(&self, text: &str) -> Vec<ParsedToolCall> {
        let mut results = Vec::new();

        for cap in self.markdown_tool_regex.captures_iter(text) {
            if let (Some(name), Some(content)) = (cap.get(1), cap.get(2)) {
                let name = name.as_str().to_string();
                let content = content.as_str().trim();

                let arguments = if content.starts_with('{') {
                    serde_json::from_str::<HashMap<String, serde_json::Value>>(content)
                        .or_else(|_| {
                            let fixed = Self::fix_windows_paths_in_json(content);
                            serde_json::from_str::<HashMap<String, serde_json::Value>>(&fixed)
                        })
                        .unwrap_or_default()
                } else {
                    self.parse_tool_content(content)
                };

                results.push(ParsedToolCall::new(
                    Uuid::new_v4().to_string(),
                    name,
                    arguments,
                ));
            }
        }

        results
    }

    /// Parse function format: tool_name({...})
    fn parse_function_format(&self, text: &str) -> Vec<ParsedToolCall> {
        let mut results = Vec::new();

        // List of known tool names to avoid false positives
        let known_tools = [
            "read_file", "write_file", "edit_file", "list_directory",
            "glob", "grep", "bash", "search", "find", "delete",
            "pdf_to_txt", "csv_to_docx", "format_liasse_fiscale",
            "web_fetch", "web_search"
        ];

        for cap in self.function_call_regex.captures_iter(text) {
            if let (Some(name), Some(json_content)) = (cap.get(1), cap.get(2)) {
                let name_str = name.as_str();

                // Only parse if it's a known tool name (avoid false positives)
                if known_tools.contains(&name_str) {
                    let content = json_content.as_str().trim();

                    let arguments = serde_json::from_str::<HashMap<String, serde_json::Value>>(content)
                        .or_else(|_| {
                            let fixed = Self::fix_windows_paths_in_json(content);
                            serde_json::from_str::<HashMap<String, serde_json::Value>>(&fixed)
                        });

                    if let Ok(arguments) = arguments {
                        results.push(ParsedToolCall::new(
                            Uuid::new_v4().to_string(),
                            name_str.to_string(),
                            arguments,
                        ));
                    }
                }
            }
        }

        results
    }

    /// Fix Windows paths in JSON strings by escaping backslashes
    fn fix_windows_paths_in_json(json_str: &str) -> String {
        // Pattern: look for paths like C:\... or \\?\ and escape the backslashes
        let mut result = String::with_capacity(json_str.len() * 2);
        let mut chars = json_str.chars().peekable();
        let mut in_string = false;

        while let Some(c) = chars.next() {
            if c == '"' && !result.ends_with('\\') {
                in_string = !in_string;
                result.push(c);
            } else if c == '\\' && in_string {
                // Check if this is already an escape sequence
                if let Some(&next) = chars.peek() {
                    match next {
                        // Valid JSON escape sequences
                        '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u' => {
                            result.push(c);
                        }
                        // Not a valid escape - this is likely a Windows path, escape the backslash
                        _ => {
                            result.push('\\');
                            result.push('\\');
                        }
                    }
                } else {
                    result.push(c);
                }
            } else {
                result.push(c);
            }
        }
        result
    }

    /// Parse the content inside a tool tag
    fn parse_tool_content(&self, content: &str) -> HashMap<String, serde_json::Value> {
        let content = content.trim();

        // Try JSON first
        if content.starts_with('{') {
            // First try as-is
            if let Ok(map) = serde_json::from_str::<HashMap<String, serde_json::Value>>(content) {
                return map;
            }
            // Try fixing Windows paths
            let fixed = Self::fix_windows_paths_in_json(content);
            if let Ok(map) = serde_json::from_str::<HashMap<String, serde_json::Value>>(&fixed) {
                return map;
            }
        }

        // Try XML-style tags like <path>/some/path</path>
        let mut result = HashMap::new();
        // Note: We capture the closing tag name separately and verify match in code
        // because the regex crate doesn't support backreferences

        for cap in self.xml_tag_regex.captures_iter(content) {
            if let (Some(open_tag), Some(value), Some(close_tag)) = (cap.get(1), cap.get(2), cap.get(3)) {
                // Only accept if opening and closing tags match
                if open_tag.as_str() == close_tag.as_str() {
                    let value_str = value.as_str().trim();
                    // Try to parse as JSON value, otherwise treat as string
                    let json_value = serde_json::from_str(value_str)
                        .unwrap_or_else(|_| serde_json::Value::String(value_str.to_string()));
                    result.insert(open_tag.as_str().to_string(), json_value);
                }
            }
        }

        // Try key="value" attribute style if no XML tags found
        if result.is_empty() {
            for cap in self.attr_regex.captures_iter(content) {
                if let Some(key) = cap.get(1) {
                    let value = cap.get(2).or_else(|| cap.get(3)).map(|m| m.as_str());
                    if let Some(value) = value {
                        result.insert(
                            key.as_str().to_string(),
                            serde_json::Value::String(value.to_string()),
                        );
                    }
                }
            }
        }

        result
    }

    /// Check if text contains tool calls (any format)
    pub fn contains_tool_calls(&self, text: &str) -> bool {
        self.tool_tag_regex.is_match(text)
            || self.emoji_tool_regex.is_match(text)
            || self.markdown_tool_regex.is_match(text)
            || self.contains_function_call(text)
    }

    /// Check if text contains a function call format for known tools
    fn contains_function_call(&self, text: &str) -> bool {
        let known_tools = [
            "read_file", "write_file", "edit_file", "list_directory",
            "glob", "grep", "bash", "search", "find", "delete",
            "pdf_to_txt", "csv_to_docx", "format_liasse_fiscale",
            "web_fetch", "web_search"
        ];

        for cap in self.function_call_regex.captures_iter(text) {
            if let Some(name) = cap.get(1) {
                if known_tools.contains(&name.as_str()) {
                    return true;
                }
            }
        }
        false
    }

    /// Find the first tool call match position across all formats
    fn find_first_tool_match(&self, text: &str) -> Option<(usize, usize)> {
        let mut first_match: Option<(usize, usize)> = None;

        // Check XML format
        if let Some(m) = self.tool_tag_regex.find(text) {
            first_match = Some((m.start(), m.end()));
        }

        // Check emoji format
        if let Some(m) = self.emoji_tool_regex.find(text) {
            if first_match.is_none() || m.start() < first_match.unwrap().0 {
                first_match = Some((m.start(), m.end()));
            }
        }

        // Check markdown format
        if let Some(m) = self.markdown_tool_regex.find(text) {
            if first_match.is_none() || m.start() < first_match.unwrap().0 {
                first_match = Some((m.start(), m.end()));
            }
        }

        // Check function format (only for known tools)
        let known_tools = [
            "read_file", "write_file", "edit_file", "list_directory",
            "glob", "grep", "bash", "search", "find", "delete",
            "pdf_to_txt", "csv_to_docx", "format_liasse_fiscale",
            "web_fetch", "web_search"
        ];
        for cap in self.function_call_regex.captures_iter(text) {
            if let (Some(full_match), Some(name)) = (cap.get(0), cap.get(1)) {
                if known_tools.contains(&name.as_str()) {
                    if first_match.is_none() || full_match.start() < first_match.unwrap().0 {
                        first_match = Some((full_match.start(), full_match.end()));
                    }
                    break;
                }
            }
        }

        first_match
    }

    /// Find the last tool call match position across all formats
    fn find_last_tool_end(&self, text: &str) -> usize {
        let mut last_end = 0;

        // Check XML format
        for m in self.tool_tag_regex.find_iter(text) {
            if m.end() > last_end {
                last_end = m.end();
            }
        }

        // Check emoji format
        for m in self.emoji_tool_regex.find_iter(text) {
            if m.end() > last_end {
                last_end = m.end();
            }
        }

        // Check markdown format
        for m in self.markdown_tool_regex.find_iter(text) {
            if m.end() > last_end {
                last_end = m.end();
            }
        }

        // Check function format (only for known tools)
        let known_tools = [
            "read_file", "write_file", "edit_file", "list_directory",
            "glob", "grep", "bash", "search", "find", "delete",
            "pdf_to_txt", "csv_to_docx", "format_liasse_fiscale",
            "web_fetch", "web_search"
        ];
        for cap in self.function_call_regex.captures_iter(text) {
            if let (Some(full_match), Some(name)) = (cap.get(0), cap.get(1)) {
                if known_tools.contains(&name.as_str()) && full_match.end() > last_end {
                    last_end = full_match.end();
                }
            }
        }

        last_end
    }

    /// Extract text before any tool calls
    pub fn extract_text_before_tools(&self, text: &str) -> String {
        if let Some((start, _)) = self.find_first_tool_match(text) {
            text[..start].trim().to_string()
        } else {
            text.to_string()
        }
    }

    /// Extract text after tool calls (for continuing conversation)
    pub fn extract_text_after_tools(&self, text: &str) -> String {
        let last_end = self.find_last_tool_end(text);
        if last_end > 0 {
            text[last_end..].trim().to_string()
        } else {
            String::new()
        }
    }
}

/// Generate a system prompt that instructs the model to use tool tags
pub fn generate_tool_prompt(tool_definitions: &[crate::llm::types::ToolDefinition]) -> String {
    let mut prompt = String::from(
        r#"Tu es Makima, un assistant de programmation expert et serviable.

## OUTILS DISPONIBLES

"#,
    );

    for tool in tool_definitions {
        prompt.push_str(&format!(
            "### {}\n{}\n\nParametres:\n```json\n{}\n```\n\n",
            tool.function.name,
            tool.function.description,
            serde_json::to_string_pretty(&tool.function.parameters).unwrap_or_default()
        ));
    }

    prompt.push_str(
        r#"## FORMAT OBLIGATOIRE POUR LES APPELS D'OUTILS

Pour utiliser un outil, tu DOIS utiliser ce format XML exact:

<tool name="nom_outil">
{"parametre": "valeur"}
</tool>

### EXEMPLES CORRECTS:

Exemple 1 - Lister un repertoire:
<tool name="list_directory">
{"path": "."}
</tool>

Exemple 2 - Lire un fichier:
<tool name="read_file">
{"path": "src/main.rs"}
</tool>

Exemple 3 - Rechercher des fichiers:
<tool name="glob">
{"pattern": "**/*.rs"}
</tool>

Exemple 4 - Chercher du texte:
<tool name="grep">
{"pattern": "TODO", "path": "src/"}
</tool>

### IMPORTANT - FORMATS INTERDITS:

N'utilise JAMAIS ces formats alternatifs:
- ❌ Emojis: 🔧 tool_name> {...}
- ❌ Markdown: ```tool:name {...} ```
- ❌ Fonction: tool_name({...})
- ❌ Autre format non-XML

## DIRECTIVES:

1. Explique toujours ce que tu vas faire AVANT d'utiliser un outil
2. Utilise read_file pour examiner un fichier avant de le modifier
3. Utilise glob pour trouver des fichiers quand tu ne connais pas le chemin exact
4. Utilise grep pour rechercher du contenu specifique dans les fichiers
5. Apres avoir recu le resultat d'un outil, analyse-le et continue a aider l'utilisateur
6. Demande confirmation avant les operations destructrices (suppression, ecrasement)

## RAPPEL FORMAT:

Le seul format accepte est:
<tool name="nom_outil">
{"parametre": "valeur"}
</tool>

Respecte EXACTEMENT ce format pour que tes appels d'outils fonctionnent.
"#,
    );

    prompt
}

/// Generate an optimized system prompt for Akari tools (灯)
/// Encourages native function calling instead of XML fallback
pub fn generate_akari_prompt(tool_definitions: &[crate::llm::types::ToolDefinition]) -> String {
    let tool_names: Vec<&str> = tool_definitions.iter().map(|t| t.function.name.as_str()).collect();

    let mut prompt = String::from(
        r#"Tu es Makima (灯 mode), un assistant de programmation expert.

## PRINCIPES

1. **Lire avant modifier** — Toujours examiner un fichier avec read_file avant de l'editer
2. **Explorer avant agir** — Utiliser glob/grep pour comprendre la structure avant de modifier
3. **Etre concis** — Reponses courtes et directes, pas de bavardage inutile
4. **Un outil a la fois** — Appeler un seul outil, attendre le resultat, puis continuer

## OUTILS DISPONIBLES

"#,
    );

    for tool in tool_definitions {
        prompt.push_str(&format!(
            "- **{}**: {}\n",
            tool.function.name, tool.function.description
        ));
    }

    prompt.push_str(&format!(
        r#"
## UTILISATION DES OUTILS

Tu disposes du function calling natif. Appelle les outils directement via le format standard tool_calls.

Si le function calling natif ne fonctionne pas, utilise ce format XML:
<tool name="nom_outil">
{{"parametre": "valeur"}}
</tool>

## WORKFLOW TYPE

1. L'utilisateur demande quelque chose
2. Tu explores avec glob/grep/read_file
3. Tu comprends le contexte
4. Tu agis avec edit_file/write_file/bash
5. Tu verifies le resultat

## CONTEXTE

- Systeme: Windows
- Outils disponibles: {}
- Mode: Akari (灯) — outils optimises pour GLM-4.6V
"#,
        tool_names.join(", ")
    ));

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_json_content() {
        let parser = ToolParser::new();
        let text = r#"Let me read that file for you.

<tool name="read_file">
{"path": "/home/user/test.txt"}
</tool>"#;

        let calls = parser.parse_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].get_string("path"), Some("/home/user/test.txt".to_string()));
    }

    #[test]
    fn test_parse_xml_content() {
        let parser = ToolParser::new();
        let text = r#"<tool name="edit_file">
  <path>/test.txt</path>
  <old_string>hello</old_string>
  <new_string>world</new_string>
</tool>"#;

        let calls = parser.parse_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "edit_file");
        assert_eq!(calls[0].get_string("path"), Some("/test.txt".to_string()));
        assert_eq!(calls[0].get_string("old_string"), Some("hello".to_string()));
        assert_eq!(calls[0].get_string("new_string"), Some("world".to_string()));
    }

    #[test]
    fn test_extract_text_before_tools() {
        let parser = ToolParser::new();
        let text = "I'll read the file now.\n\n<tool name=\"read_file\">{}</tool>";

        let before = parser.extract_text_before_tools(text);
        assert_eq!(before, "I'll read the file now.");
    }

    #[test]
    fn test_parse_emoji_format() {
        let parser = ToolParser::new();
        let text = r#"Je vais lister le repertoire.

🔧 list_directory>
{ "path": "." }
"#;

        let calls = parser.parse_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "list_directory");
        assert_eq!(calls[0].get_string("path"), Some(".".to_string()));
    }

    #[test]
    fn test_parse_emoji_format_without_arrow() {
        let parser = ToolParser::new();
        let text = r#"🔧 read_file {"path": "test.txt"}"#;

        let calls = parser.parse_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].get_string("path"), Some("test.txt".to_string()));
    }

    #[test]
    fn test_parse_markdown_format() {
        let parser = ToolParser::new();
        let text = r#"Voici l'appel d'outil:

```tool:glob
{"pattern": "**/*.rs"}
```
"#;

        let calls = parser.parse_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "glob");
        assert_eq!(calls[0].get_string("pattern"), Some("**/*.rs".to_string()));
    }

    #[test]
    fn test_parse_function_format() {
        let parser = ToolParser::new();
        let text = r#"Je vais utiliser grep({"pattern": "TODO", "path": "src/"})"#;

        let calls = parser.parse_from_text(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "grep");
        assert_eq!(calls[0].get_string("pattern"), Some("TODO".to_string()));
        assert_eq!(calls[0].get_string("path"), Some("src/".to_string()));
    }

    #[test]
    fn test_function_format_ignores_unknown_functions() {
        let parser = ToolParser::new();
        // This should NOT be parsed as a tool call because "println" is not a known tool
        let text = r#"println({"message": "hello"})"#;

        let calls = parser.parse_from_text(text);
        assert_eq!(calls.len(), 0);
    }

    #[test]
    fn test_contains_tool_calls_all_formats() {
        let parser = ToolParser::new();

        // XML format
        assert!(parser.contains_tool_calls(r#"<tool name="read_file">{"path": "x"}</tool>"#));

        // Emoji format
        assert!(parser.contains_tool_calls(r#"🔧 list_directory> {"path": "."}"#));

        // Markdown format
        assert!(parser.contains_tool_calls("```tool:glob\n{\"pattern\": \"*\"}\n```"));

        // Function format (known tool)
        assert!(parser.contains_tool_calls(r#"read_file({"path": "x"})"#));

        // Function format (unknown function - should not match)
        assert!(!parser.contains_tool_calls(r#"unknown_func({"x": 1})"#));

        // No tool calls
        assert!(!parser.contains_tool_calls("Just some regular text"));
    }

    #[test]
    fn test_xml_format_takes_priority() {
        let parser = ToolParser::new();
        // If both XML and another format are present, XML should be parsed
        let text = r#"<tool name="read_file">{"path": "correct.txt"}</tool>
🔧 list_directory> {"path": "wrong"}"#;

        let calls = parser.parse_from_text(text);
        // Should only get the XML one since we return early when XML matches
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "read_file");
        assert_eq!(calls[0].get_string("path"), Some("correct.txt".to_string()));
    }

    #[test]
    fn test_extract_text_before_emoji_tool() {
        let parser = ToolParser::new();
        let text = "Je vais lister le repertoire.\n\n🔧 list_directory> {\"path\": \".\"}";

        let before = parser.extract_text_before_tools(text);
        assert_eq!(before, "Je vais lister le repertoire.");
    }
}
