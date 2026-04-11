# Makima — Local Coding Assistant

> Claude Code, but local. Zero cloud. Zero subscription. Your machine, your model, your data.

A complete coding assistant built in Rust, powered by LM Studio's local LLMs. CLI REPL + Web interface + WebSocket streaming + 13 tools + PDF OCR pipeline. **11,785 lines of Rust.** Runs entirely on your machine.

---

## Why Makima exists

Cloud AI assistants are powerful but:
- Your code leaves your machine
- You pay per token
- You depend on API availability
- You can't customize the model

Makima solves all of that by connecting to **LM Studio** running locally:

```mermaid
graph LR
    subgraph LOCAL["Your Machine — Everything Local"]
        USER["You"]
        MAKIMA["Makima<br/><i>11,785 lines Rust<br/>CLI + Web + Tools</i>"]
        LMS["LM Studio<br/><i>Any model<br/>Qwen, GLM, Llama...<br/>localhost:1234</i>"]
        FILES["Your Code<br/><i>Read, write, edit<br/>Search, execute<br/>Never leaves disk</i>"]
    end

    USER -->|"Ask"| MAKIMA
    MAKIMA -->|"OpenAI API"| LMS
    LMS -->|"Streaming SSE"| MAKIMA
    MAKIMA -->|"Tools"| FILES
    MAKIMA -->|"Answer"| USER

    style LOCAL fill:#0a2a1a,stroke:#44cc88,color:#44cc88
    style MAKIMA fill:#12121e,stroke:#f0c040,color:#c8ccd4
    style LMS fill:#1a1a2e,stroke:#8888cc,color:#c8ccd4
```

**Zero data leaves your machine. Zero tokens billed. Zero internet required.**

---

## Architecture

```mermaid
graph TB
    subgraph MAKIMA["Makima — 11,785 lines of Rust"]
        subgraph CLI["CLI Layer — 2,591 lines"]
            REPL["repl.rs (803)<br/><i>Main REPL loop<br/>Command handling<br/>Mode switching</i>"]
            UI["ui.rs (1,150)<br/><i>Fixed panel UI<br/>Status bar, prompt<br/>Token display</i>"]
            CONFIRM["confirm.rs (329)<br/><i>Tool confirmation<br/>User approval flow</i>"]
            EVENTS["tool_events.rs (30)<br/>tool_prompts.rs (236)"]
        end

        subgraph LLM["LLM Layer — 1,677 lines"]
            CLIENT["client.rs (420)<br/><i>HTTP client<br/>SSE streaming<br/>OCR via vision API</i>"]
            PARSER["tool_parser.rs (757)<br/><i>Dual-mode parsing<br/>Native OpenAI format<br/>+ XML fallback</i>"]
            STREAM["streaming.rs (131)<br/><i>Stream accumulation<br/>Chunk processing</i>"]
            TYPES["types.rs (358)<br/><i>Message, Tool, Role<br/>Type definitions</i>"]
        end

        subgraph TOOLS["Tool Layer — 3,501 lines"]
            FILEOPS["file_ops.rs (521)<br/><i>read, write, edit,<br/>delete, list_directory</i>"]
            SEARCH["glob.rs (127) + grep.rs (198)<br/><i>Pattern matching<br/>Regex search</i>"]
            BASH_T["bash.rs (171)<br/><i>Shell execution<br/>With timeout</i>"]
            PDF["pdf_to_txt.rs (675)<br/>pdf_common.rs (494)<br/><i>Multi-stage OCR<br/>4 extraction strategies</i>"]
            CSV["csv_to_docx.rs (242)<br/><i>CSV → Word document</i>"]
            LIASSE["format_liasse.rs (918)<br/><i>Financial statement<br/>formatter</i>"]
            AKARI["akari_tools.rs (1,131)<br/><i>Optimized toolset<br/>for vision models</i>"]
            REG["registry.rs (224)<br/>executor.rs (178)<br/><i>Tool trait + registry</i>"]
        end

        subgraph WEB["Web Layer — 847 lines"]
            SERVER["server.rs (153)<br/><i>Axum HTTP server</i>"]
            ROUTES["routes.rs (242)<br/><i>REST API endpoints</i>"]
            WS["websocket.rs (445)<br/><i>Real-time streaming<br/>via WebSocket</i>"]
        end

        subgraph CTX["Context Layer — 402 lines"]
            CONV["conversation.rs (241)<br/><i>Message history<br/>Context management</i>"]
            PROJ["project.rs (154)<br/><i>Working directory<br/>Project detection</i>"]
        end
    end

    REPL --> CLIENT
    CLIENT --> LMS_EXT["LM Studio<br/>localhost:1234"]
    CLIENT --> PARSER
    PARSER --> TOOLS
    WS --> CLIENT
    ROUTES --> CLIENT

    style MAKIMA fill:#0a0a12,stroke:#f0c040,color:#f0c040
    style CLI fill:#12121e,stroke:#8888cc,color:#8888cc
    style LLM fill:#0a2a1a,stroke:#44cc88,color:#44cc88
    style TOOLS fill:#2a1a10,stroke:#ff8866,color:#ff8866
    style WEB fill:#1a0a2a,stroke:#cc88cc,color:#cc88cc
    style CTX fill:#1a1a10,stroke:#f0c040,color:#f0c040
```

---

## The Tool System

13 tools, all implementing the `Tool` trait:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, args: &ParsedToolCall) -> Result<ToolResult>;
    fn requires_confirmation(&self) -> bool { false }
}
```

```mermaid
graph TB
    subgraph TOOLS["13 Tools"]
        subgraph FILE["File Operations"]
            T1["read_file<br/><i>Read any file</i>"]
            T2["write_file<br/><i>Create or overwrite</i>"]
            T3["edit_file<br/><i>Surgical string replace</i>"]
            T4["delete<br/><i>Remove file/dir</i>"]
            T5["list_directory<br/><i>List contents</i>"]
        end
        subgraph SRCH["Search"]
            T6["glob<br/><i>Pattern matching<br/>**/*.rs</i>"]
            T7["grep<br/><i>Regex search<br/>across files</i>"]
        end
        subgraph EXEC["Execution"]
            T8["bash<br/><i>Shell commands<br/>with timeout</i>"]
        end
        subgraph CONVERT["Conversion"]
            T9["csv_to_docx<br/><i>CSV → Word</i>"]
            T10["pdf_to_txt<br/><i>PDF → Text<br/>with OCR</i>"]
            T11["format_liasse<br/><i>Financial statements</i>"]
        end
    end

    style FILE fill:#0a2a1a,stroke:#44cc88,color:#44cc88
    style SRCH fill:#1a1a2e,stroke:#8888cc,color:#8888cc
    style EXEC fill:#2a1a10,stroke:#ff8866,color:#ff8866
    style CONVERT fill:#1a0a2a,stroke:#cc88cc,color:#cc88cc
```

### Tool confirmation

Dangerous tools (write, delete, bash) require user confirmation:

```
🔧 bash: rm -rf old_folder/
   Confirmer ? [o/N] _
```

Safe tools (read, glob, grep) execute immediately.

---

## Dual-Mode Tool Parsing

Not all local models support OpenAI's native function calling. Makima handles both:

```mermaid
graph LR
    subgraph PARSE["Tool Parser (757 lines)"]
        INPUT["LLM Response"]
        CHECK{"Native tool_calls<br/>in API response?"}
        NATIVE["Parse OpenAI format<br/><i>tool_calls array<br/>function name + args</i>"]
        XML["Parse XML fallback<br/><i>&lt;tool name='...'&gt;<br/>arg1: value<br/>&lt;/tool&gt;</i>"]
        OUTPUT["Parsed Tool Call<br/><i>name + arguments<br/>ready to execute</i>"]
    end

    INPUT --> CHECK
    CHECK -->|"Yes"| NATIVE --> OUTPUT
    CHECK -->|"No"| XML --> OUTPUT

    style PARSE fill:#12121e,stroke:#f0c040,color:#f0c040
```

This means Makima works with **any LM Studio model** — even those without function calling support.

---

## PDF OCR Pipeline

The `pdf_to_txt` tool has a 4-stage extraction strategy:

```mermaid
graph TB
    subgraph OCR["PDF → Text Pipeline (1,169 lines)"]
        PDF["Input PDF"]
        S1["Stage 1: Native text<br/><i>pdf_extract crate<br/>Fast, no GPU needed</i>"]
        S2["Stage 2: Pdfium render<br/><i>Render pages to images<br/>Vector PDF support</i>"]
        S3["Stage 3: Embedded images<br/><i>lopdf crate<br/>Extract JPEG/PNG</i>"]
        S4["Stage 4: Vision OCR<br/><i>Send images to LM Studio<br/>GLM-4.6V reads the text</i>"]
        OUT["Extracted Text"]
    end

    PDF --> S1
    S1 -->|"No text? Scan PDF?"| S2
    S2 -->|"No pdfium?"| S3
    S3 --> S4 --> OUT
    S1 -->|"Text found"| OUT

    style OCR fill:#2a1a10,stroke:#ff8866,color:#ff8866
    style S4 fill:#0a2a1a,stroke:#44cc88,color:#c8ccd4
```

**The magic:** When a PDF is a scan (no embedded text), Makima renders it to images and sends them to a **vision-capable model** (like GLM-4.6V) running in LM Studio. The model reads the image and returns the text. **100% local OCR, no Tesseract, no cloud API.**

---

## Two Execution Modes

```
Shift+Tab or /plan /edit to switch:

┌─────────────────────────────────────┐
│  MODE PLAN                          │
│  Tools are SHOWN but NOT executed   │
│  Safe exploration, dry run          │
└─────────────────────────────────────┘

┌─────────────────────────────────────┐
│  MODE EDIT                          │
│  Tools ARE executed                 │
│  With confirmation for dangerous    │
│  operations (write, delete, bash)   │
└─────────────────────────────────────┘
```

---

## CLI Interface

```
┌──────────────────────────────────────────────┐
│  MAKIMA v0.1.1 — Local Coding Assistant      │
│  Model: qwen2.5-coder-14b  │  Mode: EDIT    │
│  Working dir: ./my_project                   │
├──────────────────────────────────────────────┤
│                                              │
│  > Read main.rs and explain the architecture │
│                                              │
│  📖 read_file("src/main.rs")                 │
│  [reading 474 lines...]                      │
│                                              │
│  The architecture follows a modular pattern: │
│  ...                                         │
│                                              │
├──────────────────────────────────────────────┤
│  Tokens: 2,847 │ Tools: 3 │ Uptime: 00:05   │
└──────────────────────────────────────────────┘
```

### REPL Commands

| Command | Description |
|---------|-------------|
| `/aide` | Show help |
| `/effacer` | Clear conversation history |
| `/nouveau` | Start new conversation |
| `/espace` | Change working directory |
| `/outils` | List available tools |
| `/plan` | Switch to Plan mode |
| `/edit` | Switch to Edit mode |
| `/quitter` | Quit |

---

## Web Interface

Makima also runs as a web server with WebSocket streaming:

```bash
makima serve --port 3000
# Open http://localhost:3000
```

Real-time streaming via WebSocket — see tokens appear as they're generated, just like a cloud assistant, but **100% local**.

---

## Makima vs Claude Code — Honest Comparison

Makima was built before Claude Code existed as a public CLI. They solve the same problem differently.

```mermaid
graph LR
    subgraph CLOUD["Cloud Approach — Claude Code"]
        CC_USER["Developer"]
        CC_CLI["Claude Code CLI"]
        CC_API["Anthropic Cloud<br/><i>Claude Opus 4.6<br/>1M tokens context</i>"]
        CC_USER --> CC_CLI -->|"Code sent<br/>to cloud"| CC_API
    end

    subgraph LOCAL["Local Approach — Makima"]
        M_USER["Developer"]
        M_CLI["Makima CLI/Web"]
        M_LMS["LM Studio<br/><i>Any model<br/>localhost:1234</i>"]
        M_USER --> M_CLI -->|"Everything<br/>stays local"| M_LMS
    end

    style CLOUD fill:#1a1a2e,stroke:#8888cc,color:#8888cc
    style LOCAL fill:#0a2a1a,stroke:#44cc88,color:#44cc88
```

### Feature comparison

| Feature | Claude Code (Anthropic) | **Makima** (ours) |
|---------|-------------------------|-------------------|
| **Where it runs** | Cloud | **Local (your machine)** |
| **Model** | Claude only (proprietary) | **Any model** (Qwen, GLM, Llama, Mistral...) |
| **Model quality** | Superior (Opus 4.6, 1M context) | Good (14B local, 8-32K context) |
| **Cost** | ~$20+/month | **Free forever** |
| **Data privacy** | Code sent to Anthropic servers | **Never leaves your disk** |
| **Offline** | No (requires internet) | **Yes (fully offline)** |
| **Custom model** | No | **Yes (any GGUF via LM Studio)** |
| **Tool count** | 15+ tools | **13 tools** |
| **PDF OCR** | No native support | **Yes (4-stage pipeline + vision model)** |
| **Web interface** | No (CLI only) | **Yes (Axum + WebSocket streaming)** |
| **IDE integration** | VS Code, JetBrains | Not yet |
| **MCP support** | Yes | Not yet |
| **Hooks** | Yes | Not yet |
| **Auto-memory** | Yes (key-value, cross-session) | Not yet (planned) |
| **Sub-agents** | Yes (parallel) | Not yet |
| **Context window** | 1M tokens | 8-32K (model dependent) |
| **Confirmation flow** | Yes | **Yes** |
| **Open source** | No | **Yes (MIT, 11,785 lines)** |
| **Dual-mode parsing** | Not needed (own API) | **Yes (native + XML fallback)** |

### Where Makima wins

- **Privacy** — Your code never leaves your machine. Period.
- **Cost** — Free. No subscription, no API key, no usage limits.
- **Offline** — Works without internet. Train, plane, cabin in the woods.
- **Model freedom** — Try Qwen today, switch to Llama tomorrow. Your choice.
- **PDF OCR** — Multi-stage pipeline with vision model. Claude Code can't do this.
- **Web UI** — Real-time WebSocket streaming in a browser. Claude Code is CLI-only.
- **Open source** — Read every line. Modify anything. Fork it.

### Where Claude Code wins

- **Model quality** — Claude Opus 4.6 is significantly more capable than any 14B local model.
- **Context** — 1M tokens vs 8-32K. Not even close.
- **Ecosystem** — MCP, hooks, IDE plugins, sub-agents, auto-memory.
- **Reliability** — Battle-tested by thousands of developers.

### The bottom line

Claude Code is a **Formula 1** — fast, powerful, expensive, needs a track (internet).

Makima is a **Land Rover** — slower, but goes anywhere, runs on anything, and you own it.

They're not competitors. They're complementary. Use Claude Code when you need power. Use Makima when you need privacy, freedom, or you're offline.

*We built Makima while using Claude Code. That says it all.*

---

## Tech Stack

| Component | Crate | Purpose |
|-----------|-------|---------|
| Async | tokio | Runtime |
| HTTP | reqwest 0.12 | LM Studio API + SSE streaming |
| CLI | clap 4 + crossterm + colored | Terminal UI |
| Web | axum 0.7 + tower-http | HTTP server + CORS |
| WebSocket | tokio-tungstenite | Real-time streaming |
| Files | glob + walkdir + regex | Search and traversal |
| PDF | pdf-extract + lopdf + pdfium-render | Multi-stage extraction |
| Vision | base64 + image | OCR via vision model |
| Serialization | serde + serde_json + toml | Config + API |
| Documents | csv + docx-rs | Format conversion |
| Static | rust-embed + mime_guess | Embedded web files |

---

## Quick Start

```bash
# 1. Install LM Studio and load a model
#    Recommended: Qwen2.5-Coder-14B or GLM-4.6V (for OCR)
#    Enable local server at localhost:1234

# 2. Build Makima
cargo build --release

# 3. Run CLI
./target/release/makima

# 4. Or run web server
./target/release/makima serve --port 3000

# 5. Or point to a specific project
./target/release/makima -e /path/to/project
```

---

## Line Count by Module

| Module | Lines | Role |
|--------|-------|------|
| cli/ | 2,591 | Terminal UI, REPL, confirmations |
| llm/ | 1,677 | LM Studio client, streaming, tool parsing |
| tools/ | 3,501 | 13 tools (file ops, search, bash, PDF, CSV) |
| web/ | 847 | Axum server, REST, WebSocket |
| context/ | 402 | Conversation history, project detection |
| config + main | 637 | Configuration, CLI args, entry point |
| bin/ (tests) | 736 | PDF/OCR test binaries |
| **Total** | **11,785** | |

---

## Credits

- **[LM Studio](https://lmstudio.ai/)** — Local LLM inference platform
- **IkarugaRS** — Architecture design, tool system, PDF pipeline concept
- **Akari (灯)** — Rust implementation, SSE streaming, dual-mode parser, vision OCR, web interface

## License

MIT — Your code stays on your machine. As it should.
