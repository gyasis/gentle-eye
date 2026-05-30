# Vision methods — which one reads the screen, and when to use it

gentle-eye can turn a captured frame into text/description several ways. They are
**not interchangeable** — they trade accuracy against privacy, cost, and speed.
This is the rule of thumb (encoded into the `read_screen_text` MCP tool description
so an agent picks correctly).

## The methods

| Method | How | Accuracy on dense/dark UI | Privacy | Cost/Speed | Use when |
|---|---|---|---|---|---|
| **OCR** — `read_screen_text` | local tesseract | **weak** (garbles dark, anti-aliased, multi-column terminals/IDEs) | **local, private** | free, fast | crisp/light UI text; quick extraction; sensitive screens |
| **Local vision** — `analyze_*` w/ Ollama (e.g. qwen2.5vl) | local model on the LAN box | good (better than OCR; not as sharp as cloud on tiny text) | **local, private** | free; slower (cold start) | private description/Q&A of your own desktop |
| **Cloud vision** — `analyze_video`/CLI `analyze` w/ Gemini | cloud model, full-res frame | **best** — accurate full-frame transcription, column-aware | ⚠ sends the image off-box | paid/quota; ~10–60 s | you need the *actual text* / a rich description and the content is shareable |
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

## Known gap

The MCP surface exposes `read_screen_text` (OCR) for stills and `analyze_video` for
videos, but **no `analyze_image` tool** (cloud vision on a still). Today the accurate
still-image path is the CLI (`gentle-eye analyze --image … --provider gemini`); adding
an `analyze_image` MCP tool would let an agent do the high-fidelity still transcription
directly. (Follow-up.)
