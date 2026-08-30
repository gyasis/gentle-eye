//! The documentation must not drift from the code.
//!
//! `docs/TOOLS.md` (agent reference) and `docs/GENTLE_EYE_GUIDE.md` (user guide)
//! are TWO documents describing ONE system. Two things that must agree are, in
//! this codebase's own history, the single most reliable way to end up with two
//! things that disagree — so the agreement is asserted rather than remembered.
//!
//! These tests read the SOURCE as the authority. When one fails, the code is
//! right and the document is stale.

use std::collections::BTreeSet;

fn read(p: &str) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("cannot read {p}: {e}"))
}

/// Every top-level CLI command the binary dispatches, from the dispatch match.
fn dispatched_commands() -> BTreeSet<String> {
    let src = read("src/bin/gentle-eye.rs");
    let start = src.find("let result = match cmd").expect("the dispatch match moved");
    let end = src[start..].find("\n    };").expect("dispatch match end") + start;
    let mut out = BTreeSet::new();
    for line in src[start..end].lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('"') {
            if let Some(name) = rest.split('"').next() {
                // Skip the help aliases and any empty capture.
                if !name.is_empty() && !name.starts_with('-') && name != "help" {
                    out.insert(name.to_string());
                }
            }
        }
    }
    out
}

/// Every MCP tool the server registers.
fn registered_mcp_tools() -> BTreeSet<String> {
    let src = read("src/mcp/server.rs");
    let mut out = BTreeSet::new();
    let mut lines = src.lines().peekable();
    while let Some(l) = lines.next() {
        if l.trim_start().starts_with("Tool::new(") {
            if let Some(next) = lines.peek() {
                let t = next.trim();
                if let Some(rest) = t.strip_prefix('"') {
                    if let Some(name) = rest.split('"').next() {
                        out.insert(name.to_string());
                    }
                }
            }
        }
    }
    out
}

/// Every command the binary dispatches must be discoverable in `--help`.
///
/// A command that exists and is not advertised is invisible: `dayflow` was
/// dispatched for an entire feature's worth of work while absent from HELP, so
/// neither a user nor an agent reading the help could find it.
#[test]
fn every_dispatched_command_appears_in_the_cli_help() {
    let src = read("src/bin/gentle-eye.rs");
    let start = src.find("const HELP").expect("HELP moved");
    // The HELP literal ends with `";` on the LAST content line, not on a
    // line of its own — parse for the closing quote-semicolon, not a bare line.
    let end = src[start..].find("\";\n").expect("HELP end") + start;
    let help = &src[start..end];

    let missing: Vec<String> = dispatched_commands()
        .into_iter()
        .filter(|c| !help.contains(&format!("gentle-eye {c}")))
        .collect();
    assert!(
        missing.is_empty(),
        "these commands are dispatched but absent from --help, so nobody can find them: {missing:?}"
    );
}

/// Every MCP tool must be listed in the agent reference.
///
/// `docs/TOOLS.md` is where an agent is pointed to learn what it can do. A tool
/// missing from it does not exist as far as that agent is concerned.
#[test]
fn every_mcp_tool_appears_in_the_agent_reference() {
    let doc = read("docs/TOOLS.md");
    let missing: Vec<String> = registered_mcp_tools()
        .into_iter()
        .filter(|t| !doc.contains(t.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "these MCP tools are registered but absent from docs/TOOLS.md: {missing:?}"
    );
}

/// The two documents must name the same source kinds.
///
/// The user guide and the agent reference describe one system. If the guide
/// promises `--window` and the reference does not mention it — or either names a
/// flag the parser does not accept — a reader is misled by a document they were
/// told to trust.
#[test]
fn both_documents_and_the_parser_agree_on_the_source_flags() {
    let guide = read("docs/GENTLE_EYE_GUIDE.md");
    let tools = read("docs/TOOLS.md");
    let parser = read("src/dayflow/source/mod.rs");

    for flag in ["--displays", "--window", "--target", "--input"] {
        let bare = flag.trim_start_matches("--");
        assert!(guide.contains(flag), "the user guide omits {flag}");
        assert!(tools.contains(flag), "the agent reference omits {flag}");
        assert!(
            parser.contains(bare),
            "{flag} is documented on both surfaces but SourceSpec knows nothing of {bare}"
        );
    }
}

/// The guide's central claim — one vision seam — must still be true.
///
/// If a feature grows a private path to a model, the spine the guide describes
/// stops being the spine, and every diagram in it becomes a lie.
#[test]
fn the_single_vision_seam_the_guide_describes_still_exists() {
    let guide = read("docs/GENTLE_EYE_GUIDE.md");
    assert!(guide.contains("VisionProvider"), "the guide no longer names the seam");

    // Both providers implement the shared trait.
    for provider in ["src/analysis/ollama.rs", "src/analysis/gemini.rs"] {
        assert!(
            read(provider).contains("impl VisionProvider for"),
            "{provider} no longer implements the shared vision trait — the guide's \
             'one vision layer' claim would be false"
        );
    }
    // And dayflow's perception routes through it rather than around it.
    assert!(
        read("src/dayflow/perception.rs").contains("VisionProvider"),
        "dayflow perception no longer goes through VisionProvider"
    );
}

/// No tracked document may contain a private host address.
///
/// This repository is PUBLIC. The config default is already guarded
/// (`config::tests::perception_endpoint_leaks_no_private_host`), but that guard
/// can only see the config value — and the leak that actually happened was a
/// real governor address written into a research note while recording a
/// measurement. Prose is where infra leaks, because nobody is looking there.
///
/// Endpoints belong in the environment. A document that needs to name one names
/// the VARIABLE.
#[test]
fn no_tracked_document_names_a_private_host() {
    let out = std::process::Command::new("git")
        .args(["ls-files", "*.md"])
        .output()
        .expect("git ls-files");
    let files = String::from_utf8_lossy(&out.stdout);

    // `10.` and `172.16.` are omitted deliberately: they collide constantly with
    // ordinary prose ("10. the next step", version numbers). The 192.168 block
    // and `.local` mDNS names are what actually leak here, and a guard that
    // cries wolf is a guard that gets deleted.
    let mut leaks = Vec::new();
    for f in files.lines() {
        let Ok(body) = std::fs::read_to_string(f) else { continue };
        for (n, line) in body.lines().enumerate() {
            // A generic RFC1918 EXAMPLE is fine; a real host is not. The
            // distinction that matters is whether it is presented as an address
            // to use, so any 192.168.x.y outside an obvious example is flagged.
            if line.contains("192.168.") && !line.contains("192.168.1.100") {
                leaks.push(format!("{f}:{}: {}", n + 1, line.trim()));
            }
            if line.contains("gyasis-Mac-Studio") || line.contains("localhost:3939") {
                leaks.push(format!("{f}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(
        leaks.is_empty(),
        "these tracked documents name a private host, and this repo is public:\n{}",
        leaks.join("\n")
    );
}
