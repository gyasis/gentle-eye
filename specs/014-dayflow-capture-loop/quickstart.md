# Quickstart — 014 Dayflow Capture Loop

## Build and test

```bash
cd ~/Documents/code/gentle-eye-dayflow
./.tooling/bin/cargo test --quiet          # NOT plain cargo
./.tooling/bin/cargo clippy --all-targets  # -D warnings via .cargo/config.toml
```

## Run the crate of tasks

The task crate is orchestrated and dispatched by **dev-kid**; a **Fable agent
gates every wave**. Setup first — `dev-kid.yml` and `.dk/tasks.md` still point at
feature 013:

```bash
# repoint dev-kid at 014 (setup phase, before any wave)
#   dev-kid.yml:  branch: 014-dayflow-capture-loop
#   .dk/context.json / .dk/tasks.md -> specs/014-dayflow-capture-loop/tasks.md

/devkid.orchestrate          # tasks.md -> execution_plan.json (waves)
/devkid.execute              # wave dispatch + task watchdog
```

Then, at each wave checkpoint, **strictly serial**:

```
build wave → Fable gate (reviews AND fixes) → WAIT → verify → next wave
```

No wave starts while another is under review.

## Watch it work, once the loop exists

```bash
# a display source (today's behaviour)
gentle-eye dayflow start --displays 0

# a single window — QA, or an AI agent's terminal
gentle-eye dayflow start --window "Ghostty"

# an input taken: a stream or capture device
gentle-eye dayflow start --input rtsp://…

gentle-eye dayflow status     # names the source and its availability
gentle-eye dayflow standup    # the day, categorised
gentle-eye dayflow ask "what was I doing at 2pm"
```

## The live check

`tests/dayflow_live.rs` is `#[ignore]`d and runs against the real governor:

```bash
GE_DAYFLOW_ENDPOINT=http://<governor-host>:8799/llm/ollama \
  ./.tooling/bin/cargo test --test dayflow_live -- --ignored --nocapture
```

It fails loudly and specifically when a precondition is missing, rather than
passing silently. 014 extends it with an input source, so the abstraction is
proven against something that was never on this machine's screen (SC-103a).
