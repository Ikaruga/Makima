# Troubleshooting / Solutions

Issues we hit while testing Makima with various LM Studio models, and how to fix them.

## 1. Model loops forever on tool calls

**Symptom :** You ask "what's in this directory ?" and Makima endlessly re-calls `list_directory` (10, 20, 30+ times) before finally giving up with a useless answer like "please give me a path".

**Cause :** Reasoning models (e.g. `google/gemma-4-e4b`) put their internal thinking in `reasoning_content` and an empty `content`. They struggle to acknowledge tool results — they keep "deciding" to re-call the same tool because their reasoning loop never converges on "the result is sufficient".

**Solution :**
- Don't use pure reasoning models for agentic loops. Reserve them for direct chat in LM Studio's UI.
- Use a model designed for agentic / function-calling workloads :
  - `qwen/qwen3.5-9b` (recommended — BFCL-V4 score 66.1, TAU2-Bench 79.1)
  - `zai-org/glm-4.6v-flash`
  - `Qwen/Qwen2.5-Coder-14B` or `32B`
  - Hermes 3 / DeepSeek-Coder-V2
- For Qwen3.5 specifically : keep **non-thinking mode** (default). Enabling thinking re-introduces the loop.

## 2. `max_tokens = 131072` makes everything slow / errors out

**Symptom :** Generation is painfully slow, or fails after a few exchanges with context length errors.

**Cause :** `max_tokens` in the OpenAI API is the **maximum length of a single response**, NOT the context window. Setting it to 131072 means "you may generate up to 131K tokens in this one reply", which :
- Burns generation budget even when the answer is short
- Can collide with the LM Studio context length (e.g. 125539) once prompt + history fills up

The init wizard previously mislabeled this option as "Fenetre de contexte" — fixed in commit `6adfcc6`.

**Solution :**
- Keep `max_tokens` between **2048 and 16384** for normal usage.
- The actual context window is set in **LM Studio** (Context Length slider when loading a model), not in Makima.
- Hot adjustment in REPL : `/max_tokens 8192`

## 3. Multi-line paste splits the input

**Symptom :** You paste a multi-line message into Makima's prompt, and only the first line is sent — the rest arrives as a separate input.

**Cause :** Crossterm raw mode interprets each `\n` in the pasted text as a `KeyCode::Enter`, submitting the partial input.

**Solution :** Bracketed paste mode is now enabled (commit pending). Pasted text — even multi-line — arrives as a single `Event::Paste`, with newlines collapsed to spaces so the input stays on one line.

## 4. Akari toolset description was misleading

**Symptom :** The init wizard described the Akari toolset as "Optimises pour GLM-4.6V", suggesting it only worked with GLM.

**Reality :** The Akari toolset is just an enriched, Claude Code-style version of the standard tools (precise schemas + `web_fetch` + `web_search`). It works with any model that supports native function calling (GLM, Gemma, Qwen, Hermes, DeepSeek, etc.).

**Fix :** Description corrected in commit `e4db2b8`.

## 5. Switching models manually was tedious

**Symptom :** You change the loaded model in LM Studio, but Makima keeps trying to use the one in `config.toml`, getting "model not found" errors.

**Solution (commit `e4e119b`) :**
- At startup, Makima auto-detects the loaded model and uses it (skipping embedding/reranker models via name heuristic).
- New REPL command `/auto` to re-sync mid-session if you swap the loaded model in LM Studio.
- `/modeles` lists what's currently loaded with text/vision tags.
- `/modele <name>` switches the text model (with existence check against LM Studio).

## Recommended setup (April 2026)

For an out-of-the-box experience that "just works" :

1. **LM Studio** : load `qwen/qwen3.5-9b` (or `Qwen2.5-Coder-14B`) for text and `zai-org/glm-4.6v-flash` for vision. Set Context Length to whatever your VRAM allows.
2. **Makima `config.toml`** :
   ```toml
   [lm_studio]
   url = "http://localhost:1234/v1"
   model = ""  # empty → auto-detect at startup
   vision_model = "zai-org/glm-4.6v-flash"
   max_tokens = 4096
   temperature = 0.7
   ```
3. Launch Makima → it picks up the loaded text model automatically. OCR / PDF tools route to GLM. Done.
