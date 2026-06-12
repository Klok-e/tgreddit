#!/usr/bin/env bash
set -euo pipefail

ISSUE="${1:?Usage: scripts/afk-codex-once.sh <issue-number>}"

mkdir -p .codex

if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "Working tree is dirty. Refusing to start."
  exit 1
fi

ISSUE_TEXT="$(gh issue view "$ISSUE" --comments)"

codex exec \
  --sandbox workspace-write \
  --output-last-message ".codex/issue-${ISSUE}-result.md" \
  "$(cat <<PROMPT
You are an AFK coding agent working on GitHub issue #${ISSUE}.

Your job:
1. Read the issue and Agent Brief below.
2. Implement exactly one vertical slice.
3. Use TDD:
   - write one behavior-focused test through a public interface
   - implement the smallest change to pass
   - repeat only for the listed acceptance criteria
4. Run the repo's normal validation commands. Discover them from package scripts, README, or existing CI config.
5. Commit only if all relevant validation passes.
6. If blocked, ambiguous, or unsafe, do not commit. Write .codex/issue-${ISSUE}-blocked.md explaining what is missing.

Rules:
- Do not implement out-of-scope features.
- Do not perform broad refactors.
- Do not change unrelated behavior.
- Do not use external services unless the repo already requires them for local tests.
- Prefer behavior/integration tests over implementation-detail tests.
- Keep the final answer short: changed files, tests run, commit hash if committed, or blocked reason.

Issue and comments:

${ISSUE_TEXT}
PROMPT
)"