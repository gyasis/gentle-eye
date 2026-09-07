# gentle-eye — the guide

**gentle-eye gives a machine sight.** Everything in it is one of four things:
a way to **get pixels**, a way to **give those pixels structure**, a way to
**understand them**, or a way for a **human to point at something** in them.

That is the whole product. Each tool below is a different answer to "which
pixels, and what do you want to know about them?"

> Looking for exact commands, flags and JSON shapes? `docs/TOOLS.md` is the
> lookup reference (it serves agents; this file serves people). A test fails if
> the two disagree.

---

## The spine: one vision layer

Every feature that *understands* an image goes through the same seam —
`VisionProvider`, with two implementations: **ollama** (local, through the
Atelier governor) and **Gemini** (cloud). Nothing has a private path to a model.

```
   get pixels          give them structure        understand them
   ──────────          ───────────────────        ───────────────
   screenshot   ─┐     regions (WM + a11y     ─┐
   record        ├──▶  cascade → boxes,        ├──▶  VisionProvider
   capture-stream│     reading order)          │      ├─ ollama  (governed lane)
   dayflow      ─┘     target (a saved crop)  ─┘      └─ gemini  (cloud)
                                                          │
   redpen  ───── a human draws on a screenshot ───────────┘
                 (the one INBOUND channel)
```

Two cheaper tiers sit beside the models, on purpose: **tesseract** for plain OCR
(`read-text`), and **geometry** for reading order — which is never a model, because
the order boxes should be read in is a spatial fact, not an opinion.

**Which provider:** ollama through the Atelier governor is *local, free and
private* (default model `qwen2.5vl:7b`; point `OLLAMA_HOST` at the governed
lane, never at raw ollama); Gemini is *cloud, paid and the sharpest on dense
text* (default `gemini-flash-latest`, an alias on purpose — the pinned model it
replaced no longer exists). OCR and geometry cost nothing and leave the box
never. The full comparison, the models on the governed lane and how to configure
each: `docs/VISION_METHODS.md`.

---

## Which tool, for what

| You want to… | Reach for | Why that one |
|---|---|---|
| Grab one image now | `screenshot` | one-shot, optional crop |
| Record a session to video | `record` | explicit start/stop, you keep the file |
| Know what is on screen *right now* | `read-text`, `analyze` | OCR, or a VLM answer |
| Know what a **structure** looks like | `regions` | window/pane/element boxes with reading order |
| Watch **one specific thing** repeatedly | `target` | a named, normalised crop that survives a resolution change |
| Record **your whole day** and ask about it later | `dayflow` (`--displays 0,1` for every screen) | samples, summarises and answers — the only self-running one |
| **Point at** something for an agent | `redpen` | you draw; the agent reads the markup |
| Have the agent **point back** at something | `annotate` | headless boxes + labels on an image — the agent's half of the same loop |
| Split a busy screen into its **panels** | `segment` | column-activity dividers; `--read` reads each one |
| Turn a **recording into a transcript** | `frames` → `read-text` → `quality` → `merge-text` | three primitives that measure and decide nothing; you own the thresholds |
| See a live stream / capture card | `capture-stream`, `dayflow --input` | content that was never on your screen |

---

## The workflows that actually get used

### 1. "Watch one window all day, then ask about it"

The QA case: one thing under observation for hours, asked about afterwards.

```bash
gentle-eye dayflow serve --window "Firefox"     # the daemon owns the session
gentle-eye dayflow status                       # any other shell attaches to it
gentle-eye dayflow ask "did the build ever go red?"
```

A change **elsewhere on the desktop produces no sample** — the session records
that window and nothing else. Restarting the daemon on the same day resumes the
same session and records the interruption as a gap, so the hole is a stated fact
rather than a silent absence.

### 2. "Point at what I mean"

You see something wrong and drawing is faster than describing it.

```bash
redpen                       # you draw: circle it, arrow at it, box it
gentle-eye redpen-list       # the agent finds your markup
gentle-eye redpen-analyze --prompt "what am I pointing at?"
```

Your strokes arrive at the model **as text** — "green ARROW from (x,y) to (x,y)"
— so direction is understood, not merely seen. This is the inbound half of the
loop; `target` is the outbound half (the agent choosing a region to watch), and
`annotate` is how the agent answers in kind — `gentle-eye annotate --image shot.png
--out marked.png --box x,y,w,h --label "this one"` draws its own boxes for you to
look at. Both halves exist deliberately; neither replaces the other.

### 3. "Only this region matters"

```bash
gentle-eye target add dashboard --display 0 --region 0.1,0.1,0.5,0.4
gentle-eye target use dashboard
gentle-eye screenshot --target dashboard --out now.png
gentle-eye dayflow serve --target dashboard        # …or record it all day
```

The region is stored **normalised**, so it survives a resolution change. The
pixel rectangle is resolved per frame, never cached.

### 4. "Something that was never on my screen"

```bash
gentle-eye dayflow serve --input rtsp://camera.local/live
```

A stream, a capture card, a video file. This is the case that proves the
abstraction is real rather than a filter over screen capture — verified by a live
test that reads a word back out of a video that was never rendered on this
desktop.

### 5. "Turn this recording into a transcript"

A lecture, a screen-shared call, a long build. Three primitives, chained by you:

```bash
gentle-eye frames --video lesson.mkv --out ./frames --fps 2 --dedup medium   # which frames, how sharp
gentle-eye read-text --image ./frames/f_00007.png                              # read the sharp ones
gentle-eye quality reading.txt          # real content, or a reader that broke down? three ratios
gentle-eye merge-text --similarity 0.85 doc.txt reading.txt                   # one document
```

Each **answers one question and decides nothing** — no primitive picks a
threshold, judges a transcript good enough, or chains into the next. That is on
purpose: how much of a recording is new is a property of the *material*
(scrolling text changes every frame, slides do not), so the knobs are yours.
Two things bite: a frame's `timestamp_s` is ffmpeg's clock, not `index/fps`
(after dedup the index counts survivors); and `--dedup aggressive` drops a
slide whose *title* changed as a near-duplicate. The runnable recipe, with what
proves it worked: `docs/playbooks/transcribe-a-recording.md`.

---

## What dayflow actually does

Dayflow **samples**; it does not record video. Roughly one frame every three
minutes all day, or one a minute for a focused session. Then:

1. **Gate** — a frame that did not change is skipped; a frame that was *wanted*
   and could not be obtained is a **drop**, recorded as missing data. Skips and
   drops are different facts and are never conflated.
2. **Segment** — samples group into 10–15 minute windows.
3. **Perceive** — a cheap text tier reads each crop; one reasoning call
   summarises the segment. Cropping first is why it is affordable.
4. **Timeline** — entries carry *where on screen* their text came from.
5. **Retain** — old raw samples are reclaimed, but **only after they are
   summarised**. Never on age, never on disk pressure.
6. **Ask** — grounded in your own records. An empty range never reaches a model:
   with no evidence it would invent a day, and every answer carries its grounding
   so confident prose with nothing behind it stays visible.

Every gate **fails open**: on any error the sample is kept, because you cannot
re-capture yesterday.

---

## Using gentle-eye from other tools

It is a library, a CLI, an MCP server and an HTTP surface — pick by how much
integration you want to pay for.

| As… | For | Cost |
|---|---|---|
| **CLI** | any agent or script that can run a shell | none — JSON on stdout, no registration |
| **MCP server** | a coding agent, in-conversation | one registration per host |
| **HTTP** (`dayflow serve`) | driving a capture daemon, possibly on another machine | run the daemon |
| **Rust library** | embedding it | a dependency |

The CLI is the zero-install path and the reason it exists: an MCP tool must be
registered for every session, a CLI need not be. Every subcommand prints JSON on
stdout, diagnostics on stderr, and **exits 0 on a degraded-but-recoverable
state** — a non-zero exit would make every script treat a recoverable condition
as a crash.

**Where it must run natively:** capture handles are OS-specific and thread-affine,
so the *daemon* has to be native on the machine being watched. A *client* does
not — which is what lets a thin client drive a capture daemon on another box.

## What it is built on, and what that costs you

| | | The constraint it brings |
|---|---|---|
| `scrap` | screen capture | capture handles cannot cross threads |
| `x11rb` | window geometry, idle | **X11 only** — `--window` misreports elsewhere |
| `atspi` | accessibility regions | needs the a11y bus; blocked in some sandboxes |
| `rusqlite` | the timeline | one store, shared by every surface |
| `rmcp` | the MCP server | — |
| ffmpeg / ffplay / tesseract | inputs, preview, OCR | external binaries; absent → a stated failure, never a silent one |

Models run through the **Atelier governor**, which admits and evicts them so the
machine does not thrash. A cold load takes real time — measured at ~95 s — so the
first question after an idle period is slow by nature, not broken.

---

## Honest limits

Read `docs/DAYFLOW_LIMITATIONS.md` before trusting dayflow with something that
matters. It is a real ledger, kept current: items are **removed when closed**,
and the ones still open are named with what closes them.

A green `cargo test` certifies none of the live behaviour. The tests that do are
`#[ignore]`d and listed in that ledger with how to run them.
