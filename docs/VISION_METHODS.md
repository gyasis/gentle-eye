# Vision methods — which one reads the screen, and when to use it

gentle-eye can turn a captured frame into text/description several ways. They are
**not interchangeable** — they trade accuracy against privacy, cost, and speed.
This is the rule of thumb (encoded into the `read_screen_text` MCP tool description
so an agent picks correctly).

## The methods

| Method | How | Accuracy on dense/dark UI | Privacy | Cost/Speed | Use when |
|---|---|---|---|---|---|
| **Geometry** — `regions` | WM + a11y tree, reading order | n/a — boxes, not text | **local, private** | free, instant | *structure*: which panes/elements exist and in what order. Never a model: reading order is a spatial fact, not an opinion |
| **OCR** — `read_screen_text` / `read-text` | local tesseract | **weak** (garbles dark, anti-aliased, multi-column terminals/IDEs) | **local, private** | free, fast | crisp/light UI text; quick extraction; sensitive screens |
| **Local vision** — `analyze_*` w/ Ollama (default `qwen2.5vl:7b`) | local model on the LAN box, reached **through the Atelier governor** | good (better than OCR; not as sharp as cloud on tiny text) | **local, private** | free; slower (a cold load is real time) | private description/Q&A of your own desktop |
| **Cloud vision** — `analyze_video`/CLI `analyze` w/ Gemini (default `gemini-flash-latest`) | cloud model, full-res frame | **best** — accurate full-frame transcription, column-aware | ⚠ sends the image off-box | paid/quota; ~10–60 s | you need the *actual text* / a rich description and the content is shareable |
| **Agent Read (Claude)** | harness downsamples the image | layout yes, fine text no | local to the session | — | reading *structure/layout*, not verbatim text |

## Why OCR / a downscaled Read fall short on a busy screen

- The agent's `Read` **downsamples** large images to fit the vision input, so a
  3440×1440 dense screenshot loses fine text — good for *layout*, not for reading it.
- **tesseract** struggles with dark themes, small anti-aliased fonts, and tiled
  columns — the output is noisy/garbled.
- A **cloud vision model (Gemini)** handles higher effective resolution and is far
  stronger at dense screen-text transcription. Proven here: a 4-column ultrawide of
  Cursor/Claude-Code sessions transcribed accurately, column by column (11k tokens),
  where OCR returned garble.

## Recommended flow

1. **Need layout / a quick gist, or it's sensitive?** → `read_screen_text` (OCR) or
   local Ollama. Stays on-box.
2. **Need the actual text accurately, and it's shareable?** → cloud `analyze` with a
   *"transcribe all on-screen text, preserve columns"* prompt.
3. **Dense ultrawide and you want near-perfect text?** → **tile** into its columns
   and analyze each tile at full resolution (each ~860×1440 fits comfortably), then
   stitch — far better than one downscaled pass.

## Privacy rule (default)

Prefer **OCR / local vision** for your own desktop and anything sensitive. Reach for
the **cloud provider only when fidelity matters and the content is OK to share** —
it sends the frame off-box (and screen captures can contain private info: hostnames,
IPs, tokens on screen, account pages).

## Configuring the providers (verified live 2026-09-07)

Both providers implement one trait, `VisionProvider` (`src/contracts/traits.rs`).
Nothing else in the codebase has a private path to a model.

### Which provider and model a call uses

**The default provider is `ollama`.** Gemini is billed per call, so it is used
only when something names it. Each setting is resolved in this order, first
match wins:

| Setting | 1. per call | 2. environment | 3. config file (`[vision]`) | 4. built-in |
|---|---|---|---|---|
| provider | CLI `--provider`, MCP `provider` | `GENTLE_EYE_PROVIDER` | `provider` | `ollama` |
| Ollama model | CLI `--model`, MCP `model` | `GENTLE_EYE_OLLAMA_MODEL` | `ollama_model` | `qwen2.5vl:7b` |
| Gemini model | CLI `--model`, MCP `model` | `GENTLE_EYE_GEMINI_MODEL` | `gemini_model` | `gemini-flash-latest` |

`--model` (and the MCP `model` parameter) sets the model **of the provider the
call uses**: with `--provider gemini --model gemini-3.8-flash` it is the Gemini
model, with provider ollama it is the Ollama model. `--provider` and `--model`
work on `analyze`, `provider-info`, `segment --read` and `redpen-analyze`; the
MCP tools `analyze_image` and `analyze_video` both take `provider` and `model`.

```bash
gentle-eye analyze --image shot.png --prompt "what is on screen?"                       # ollama, configured model
gentle-eye analyze --image shot.png --prompt "..." --model minimax-m3:cloud             # ollama, that model
gentle-eye analyze --image shot.png --prompt "..." --provider gemini --model gemini-3.8-flash
gentle-eye analyze --help
```

**List what is live** before naming a model:

```bash
gentle-eye models                      # the active provider
gentle-eye models --provider ollama    # /api/tags, then /api/show per model; keeps capability `vision`, reports `thinking`
gentle-eye models --provider gemini    # models.list; keeps `generateContent` models, newest first
```

`models` uses metadata routes only and never sends a generation request.
`models.list` reports no release date, so "newest first" is the highest version
number in the model name, with the unversioned `-latest` aliases ahead.

**Result fields.** `model_used` is the bare model name (`minimax-m3:cloud`), so
it can be compared with `==`. For video on Ollama, `frames_analyzed` is the
number of frames the model was shown and `sheets` the number of contact sheets
they were tiled onto; both are absent where they do not apply.

### Gemini spend: usage log and burst cap

Every Gemini generation call appends one JSON line to
`~/.local/share/gemini-cli/usage.jsonl`, the log the `gemini` CLI writes, so
`gemini usage` counts gentle-eye calls too:

```json
{"ts":"…","cmd":"analyze image","cwd":"…","model":"gemini-3.8-flash","in":1684,"out":896,"thinking":1369,"usd":0.009757,"burst":"b…","tool":"gentle-eye"}
```

- Token counts come from the response's `usageMetadata` (prompt, candidates,
  thoughts). Thinking tokens bill at the output rate.
- `usd` comes from the price table in `src/analysis/gemini_usage.rs` (the
  `gemini` CLI's `PRICES`). A model with no listed price logs `"usd": null` and
  a warning on stderr; a price is never guessed. For an alias such as
  `gemini-flash-latest` the price is looked up under the `modelVersion` the API
  reports, which is then logged as `model_version`.
- With **`GEMINI_BURST_ID`** set, the line carries `"burst"`, and before each
  call the spend logged under that burst is summed and compared with `cap_usd`
  in `~/.local/share/gemini-cli/bursts/<id>.json`. At or over the cap, or with
  no readable cap file, the call is refused before it is sent. Calls with
  `"usd": null` count as 0 toward the cap, so name a priced model inside a burst.

### Gemini — cloud, paid, sharpest

| | |
|---|---|
| Auth | `GEMINI_API_KEY` or `GOOGLE_API_KEY`, sent as the **`x-goog-api-key` header** — never a `?key=` query parameter. The old query form put the key into every transport-error URL, and reqwest stringifies URLs into its error type; it was fixed 2026-09-06 (`281e36e`) and `map_http_err` now redacts `key=` / `AIza…` from any error text as a second line of defence. |
| Default model | **`gemini-flash-latest`** — `DEFAULT_GEMINI_MODEL`, defined **once** in `src/contracts/traits.rs` and referenced by `analysis/gemini.rs`, `config/mod.rs` and `models/config.rs`. |
| Why an alias, not a pin | The previous pinned default `gemini-2.0-flash` has been **deleted** by Google (`NOT_FOUND: "This model is no longer available"`), so every call taking the default was failing at runtime. An alias cannot 404; it can shift behaviour, which is the accepted trade. Override with `--model` / `GENTLE_EYE_GEMINI_MODEL` / `gemini_model` in the config file. |
| Deep tier | `DEEP_MODEL = gemini-pro-latest` (`src/analysis/gemini.rs`) — select via config when Flash is not enough. |
| Historical note | `docs/GENTLE_EYE_PRD.md`, `docs/REBUILD_OVERVIEW.md` and `specs/001-*` still name `gemini-2.0-flash`. They are the **recovered historical spec** and are left as written; the runtime is what is described here. |

Check it: `gentle-eye provider-info --provider gemini` returns
`{"model":"gemini-flash-latest","available":true}` against the real API.

### Ollama — local, free, private — through the Atelier governor

The endpoint resolves in this order, first match wins:
**`GENTLE_EYE_OLLAMA_HOST`** → `vision.ollama_host` in the config file →
`OLLAMA_HOST` / `OLLAMA_PORT` (or `OLLAMA_URL` in the library API) →
`http://localhost:11434`. The config file beats the generic `OLLAMA_HOST`, so a
shell that exports `OLLAMA_HOST` for other tools cannot pull gentle-eye off the
governor its config names. Check the result with `gentle-eye models` (`host`); model default **`qwen2.5vl:7b`**, overridable via
`--model` / `GENTLE_EYE_OLLAMA_MODEL` / `ollama_model` in the config file.

**Point it at the governor, not at raw ollama.** The Atelier governor admits,
loads and auto-unloads models under a memory budget; raw ollama on that shared
box can drive it into unified-memory swap-death. The governed lane is
`:8799/llm/ollama`, and the host is **machine-local configuration** supplied
through the environment — it is never written into this repository:

```bash
export OLLAMA_HOST="http://$ATELIER_HOST:8799/llm/ollama"     # the governed lane
gentle-eye models --provider ollama                           # the vision models live on that lane
gentle-eye analyze --image shot.png --prompt "what is on screen?"
```

Verified end to end 2026-09-07:

```
OLLAMA_HOST=http://$ATELIER_HOST:8799/llm/ollama \
  cargo test --test live_vision ollama -- --include-ignored --nocapture
→ exit 0, [OLLAMA model=qwen2.5vl:7b tokens=13], "The image is a solid orange color…", 10.7 s
```

**Vision models on the governed lane — selected by CAPABILITY, not by name.**
`POST /api/show` returns a `capabilities` list per model; a name match (`*vl*`,
`*vision*`) is the wrong filter and has produced a false "no vision model here"
before. As checked 2026-09-07 (62 models on the lane in total):

| Model | capabilities | Note |
|---|---|---|
| `qwen2.5vl:7b` | completion, vision | **the default** — fast; use it unless you need tools or thinking |
| `qwen3-vl:32b` | completion, vision, tools, thinking | thinking model — see below |
| `gemma4:31b` | completion, vision, tools, thinking | thinking model — see below |
| `gemma3:27b` | completion, vision | |
| `llava:7b` | completion, vision | |
| `moondream:latest` | completion, vision | small |

**Thinking models cost more for the same answer, and their output needs
handling.** Through ollama's `/api/generate` a thinking-capable model
(`qwen3-vl:*`, `gemma4:*`, `ornith-*`) returns its chain-of-thought *inline* in
`response`, terminated by `</think>`, ahead of the actual answer — measured at
roughly 60 % of the text on `ornith-1.5-9b` through the governor. gentle-eye's
`OllamaProvider` strips that preamble (`strip_reasoning` in
`src/analysis/ollama.rs`: splits on the *last* `</think>`, leaves non-thinking
models untouched, and never returns an empty string — a response that is
entirely reasoning is passed through as-is, because a wrong-but-present answer is
easier to debug than a blank one). Budget the extra latency and tokens; on an
OpenAI-style lane (llama.cpp `:8771`) the same models put their reasoning in
`reasoning_content` and a tight `max_tokens` yields an empty `content` — a
failure mode gentle-eye's ollama path does not have, but worth knowing if you
call the lane directly. Prefer `qwen2.5vl:7b` for perception.

A cold load through the governor is slow (~95 s measured) but normal; budget for
it rather than treating it as a fault. The `reqwest` client uses a single
`timeout_seconds` (`VisionConfig`), so set it generously for a cold model.

### Choosing, in one line each

| You want | Use |
|---|---|
| free, private, on-box, good enough | **Ollama via the governor** (`qwen2.5vl:7b`) |
| the sharpest read of dense text, content is shareable | **Gemini** (`gemini-flash-latest`; `gemini-pro-latest` for depth) |
| plain text, no model at all | **OCR** (`read-text`) |
| structure and reading order, no model at all | **geometry** (`regions`) |

## Known gap

The MCP surface exposes `read_screen_text` (OCR) for stills and `analyze_video` for
videos, but **no `analyze_image` tool** (cloud vision on a still). Today the accurate
still-image path is the CLI (`gentle-eye analyze --image … --provider gemini`); adding
an `analyze_image` MCP tool would let an agent do the high-fidelity still transcription
directly. (Follow-up.)
