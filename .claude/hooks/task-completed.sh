#!/usr/bin/env bash
# Portable shim: resolves dev-kid hooks under the CURRENT user's $HOME.
# Was an absolute symlink, which dangles on any other box (different user, a
# fresh clone, or a teammate's machine).
# Exits 0 (no-op) when dev-kid is not installed, so hooks never break a session.
target="$HOME/.dev-kid/templates/.claude/hooks/task-completed.sh"
[ -x "$target" ] || exit 0
exec "$target" "$@"
