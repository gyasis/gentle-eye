# Playbooks — harness-agnostic

Task-shaped recipes for gentle-eye, in plain markdown, using **only the CLI**.

No MCP registration. No `.claude/` format, no `.cursorrules`, no per-harness
skill file. Any agent that can run a shell can follow these — Claude Code,
Codex, Gemini CLI, Cursor, opencode, aider, or a person.

That is deliberate. An MCP tool has to be registered for every session on every
host; a CLI does not. These playbooks are the zero-install path to the same
capability, which is why they are the primary form and MCP is the convenience.

## Contract every playbook keeps

- **CLI only.** Every step is a shell command that prints JSON on stdout.
- **State what proves it worked.** Each playbook ends with a check, not a hope.
- **Name the failure.** What it looks like when it goes wrong, and what to do.
- **No secrets, no hosts.** Endpoints come from the environment; nothing here
  contains a real address.

## The playbooks

| File | Task |
|---|---|
| `watch-one-thing.md` | record one window/region all day and ask about it |
| `read-a-human-markup.md` | the user drew on their screen — read what they meant |
| `understand-this-screen.md` | one-shot: what is on screen, or in this region |
| `watch-an-input.md` | record a stream/capture card that is not on this screen |
