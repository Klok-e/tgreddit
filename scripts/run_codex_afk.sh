#!/usr/bin/env bash
set -euo pipefail

MAX_CYCLES=5

usage() {
  echo "Usage: scripts/run_codex_afk.sh" >&2
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 is required." >&2
    exit 1
  fi
}

codex_afk_exec() {
  codex \
    --sandbox workspace-write \
    --ask-for-approval never \
    exec \
    -c sandbox_workspace_write.network_access=true \
    "$@"
}

codex_afk_resume() {
  codex \
    --sandbox workspace-write \
    --ask-for-approval never \
    exec resume \
    -c sandbox_workspace_write.network_access=true \
    "$@"
}

issue_status() {
  local issue="$1"

  awk '
    /^Status:[[:space:]]*/ {
      sub(/^Status:[[:space:]]*/, "")
      sub(/[[:space:]]*$/, "")
      print
      exit
    }
  ' "$issue"
}

blocked_by_paths() {
  local issue="$1"

  awk '
    /^##[[:space:]]+Blocked by[[:space:]]*$/ {
      in_block = 1
      next
    }
    in_block && /^##[[:space:]]+/ {
      exit
    }
    in_block {
      line = $0
      sub(/\r$/, "", line)
      if (line ~ /^[[:space:]]*$/) {
        next
      }
      if (line ~ /^[[:space:]]*None([[:space:]]|$|-)/) {
        next
      }
      if (line ~ /^[[:space:]]*-[[:space:]]+/) {
        sub(/^[[:space:]]*-[[:space:]]*/, "", line)
        sub(/[[:space:]]*$/, "", line)
        if (line != "") {
          print line
        }
      }
    }
  ' "$issue"
}

is_runnable_issue() {
  local issue="$1"
  local blocker
  local blocker_status

  if [[ "$(issue_status "$issue")" != "ready-for-agent" ]]; then
    return 1
  fi

  while IFS= read -r blocker; do
    if [[ ! -f "$blocker" ]]; then
      printf '%s skipped: missing blocker %s\n' "$issue" "$blocker" >> "$SKIP_REASONS"
      return 1
    fi

    blocker_status="$(issue_status "$blocker")"
    if [[ "$blocker_status" != "complete" ]]; then
      printf '%s skipped: blocker %s is %s\n' "$issue" "$blocker" "${blocker_status:-missing-status}" >> "$SKIP_REASONS"
      return 1
    fi
  done < <(blocked_by_paths "$issue")

  return 0
}

first_runnable_issue() {
  find .scratch -path '*/issues/*.md' -type f 2>/dev/null \
    | sort \
    | while IFS= read -r issue; do
        if is_runnable_issue "$issue"; then
          printf '%s\n' "$issue"
          return 0
        fi
      done
}

issue_title() {
  local issue="$1"
  local title

  title="$(grep -m 1 -E '^#[[:space:]]+' "$issue" | sed -E 's/^#[[:space:]]+//' || true)"
  if [[ -n "$title" ]]; then
    printf '%s\n' "$title"
  else
    basename "$issue" .md
  fi
}

set_issue_status() {
  local issue="$1"
  local status="$2"
  local tmp_file="$TMPDIR/status.md"

  awk -v status="$status" '
    BEGIN { changed = 0 }
    !changed && /^Status:[[:space:]]*/ {
      print "Status: " status
      changed = 1
      next
    }
    { print }
    END {
      if (!changed) {
        print "Status: " status > "/dev/stderr"
        exit 2
      }
    }
  ' "$issue" > "$tmp_file"

  mv "$tmp_file" "$issue"
}

ensure_comments_section() {
  local issue="$1"

  if ! grep -Eq '^## Comments[[:space:]]*$' "$issue"; then
    {
      printf '\n'
      printf '## Comments\n'
    } >> "$issue"
  fi
}

append_issue_comment() {
  local issue="$1"
  local heading="$2"
  local body="$3"

  ensure_comments_section "$issue"
  {
    printf '\n'
    printf '### %s\n\n' "$heading"
    printf '%s\n' "$body"
  } >> "$issue"
}

write_schemas() {
  VERIFY_SCHEMA="$TMPDIR/verifier.schema.json"
  COMMIT_SCHEMA="$TMPDIR/commit-message.schema.json"

  cat > "$VERIFY_SCHEMA" <<'JSON'
{
  "type": "object",
  "additionalProperties": false,
  "required": ["status", "summary", "feedback", "commands_run"],
  "properties": {
    "status": { "type": "string", "enum": ["pass", "fail"] },
    "summary": { "type": "string" },
    "feedback": { "type": "string" },
    "commands_run": {
      "type": "array",
      "items": { "type": "string" }
    }
  }
}
JSON

  cat > "$COMMIT_SCHEMA" <<'JSON'
{
  "type": "object",
  "additionalProperties": false,
  "required": ["subject", "body"],
  "properties": {
    "subject": { "type": "string" },
    "body": { "type": "string" }
  }
}
JSON
}

run_initial_implementer() {
  local issue="$1"
  local title="$2"
  local out="$TMPDIR/implement-initial.jsonl"
  local last="$TMPDIR/implement-initial.txt"

  codex_afk_exec \
    --json \
    --cd "$PWD" \
    --output-last-message "$last" \
    "$(cat <<PROMPT
You are an AFK coding agent working on local issue:

${issue}

Issue title: ${title}

Your job:
1. Read the issue file and relevant repo docs.
2. Implement the issue using TDD.
3. Do not commit.
4. If blocked, ambiguous, or unsafe, stop and explain the blocker in your final message.

Rules:
- Do not implement out-of-scope features.
- Do not change unrelated behavior.
- Leave the worktree ready for an independent verifier.
PROMPT
)" | tee "$out"

  THREAD_ID="$(jq -r 'select(.type == "thread.started") | .thread_id' "$out" | sed -n '1p')"
  if [[ -z "$THREAD_ID" || "$THREAD_ID" == "null" ]]; then
    echo "Could not capture implementer thread id." >&2
    exit 1
  fi
}

resume_implementer() {
  local issue="$1"
  local cycle="$2"
  local feedback="$3"
  local out="$TMPDIR/implement-cycle-${cycle}.jsonl"
  local last="$TMPDIR/implement-cycle-${cycle}.txt"

  codex_afk_resume \
    --json \
    --output-last-message "$last" \
    "$THREAD_ID" \
    "$(cat <<PROMPT
Verifier failed cycle ${cycle} for local issue:

${issue}

Address the verifier feedback below, then leave the worktree ready for verification again.

Do not commit.
Do not change out-of-scope behavior.

Verifier feedback:

${feedback}
PROMPT
)" | tee "$out"
}

run_verifier() {
  local issue="$1"
  local title="$2"
  local cycle="$3"
  local out="$TMPDIR/verify-cycle-${cycle}.jsonl"
  local last="$TMPDIR/verify-cycle-${cycle}.json"

  if ! codex_afk_exec \
    --json \
    --cd "$PWD" \
    --output-schema "$VERIFY_SCHEMA" \
    --output-last-message "$last" \
    "$(cat <<PROMPT
You are an independent verifier for local issue:

${issue}

Issue title: ${title}

Verify the current worktree against the issue and repo rules.

Your job:
1. Read the issue file, relevant docs, and current worktree.
2. Inspect the implementation for correctness, scope, and accidental unrelated changes.
3. Run relevant validation commands yourself.
4. Do not intentionally edit files and do not commit.
5. Return pass only if the issue acceptance criteria are met, relevant validation passes, and the change is scoped.

Return JSON matching the schema:
- status: "pass" or "fail"
- summary: concise outcome
- feedback: exact fixes needed if fail, or concise rationale if pass
- commands_run: validation commands you ran
PROMPT
)" | tee "$out"; then
    cat > "$last" <<'JSON'
{
  "status": "fail",
  "summary": "Verifier command failed.",
  "feedback": "The verifier Codex run exited nonzero. Inspect temporary runner output from this invocation if available, then rerun.",
  "commands_run": []
}
JSON
  fi

  VERIFY_STATUS="$(jq -r '.status' "$last")"
  VERIFY_SUMMARY="$(jq -r '.summary' "$last")"
  VERIFY_FEEDBACK="$(jq -r '.feedback' "$last")"
  VERIFY_COMMANDS="$(jq -r '.commands_run | join(", ")' "$last")"

  if [[ "$VERIFY_STATUS" != "pass" && "$VERIFY_STATUS" != "fail" ]]; then
    echo "Verifier returned invalid status: $VERIFY_STATUS" >&2
    exit 1
  fi
}

generate_commit_message() {
  local issue="$1"
  local title="$2"
  local out="$TMPDIR/commit-message.jsonl"
  local last="$TMPDIR/commit-message.json"

  codex_afk_exec \
    --json \
    --cd "$PWD" \
    --output-schema "$COMMIT_SCHEMA" \
    --output-last-message "$last" \
    "$(cat <<PROMPT
Generate a commit message for the current staged or unstaged repository changes.

Local issue:

${issue}

Issue title: ${title}

Inspect the current diff yourself. Return JSON matching the schema:
- subject: imperative commit subject, 72 chars or fewer if practical
- body: concise commit body, or empty string if no body is needed

Do not edit files.
Do not commit.
PROMPT
)" | tee "$out"

  COMMIT_SUBJECT="$(jq -r '.subject' "$last")"
  COMMIT_BODY="$(jq -r '.body' "$last")"

  if [[ -z "$COMMIT_SUBJECT" || "$COMMIT_SUBJECT" == "null" ]]; then
    echo "Commit message agent returned empty subject." >&2
    exit 1
  fi
}

commit_changes() {
  if [[ -n "$COMMIT_BODY" && "$COMMIT_BODY" != "null" ]]; then
    git commit -m "$COMMIT_SUBJECT" -m "$COMMIT_BODY"
  else
    git commit -m "$COMMIT_SUBJECT"
  fi
}

if (($# != 0)); then
  usage
  exit 2
fi

require_cmd codex
require_cmd git
require_cmd jq

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Working tree is dirty. Refusing to start." >&2
  exit 1
fi

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

SKIP_REASONS="$TMPDIR/skipped-issues.txt"

ISSUE_PATH="$(first_runnable_issue || true)"
if [[ -z "$ISSUE_PATH" ]]; then
  echo "No runnable ready-for-agent issues found under .scratch/*/issues/*.md" >&2
  if [[ -s "$SKIP_REASONS" ]]; then
    echo >&2
    cat "$SKIP_REASONS" >&2
  fi
  exit 1
fi

THREAD_ID=""
VERIFY_STATUS=""
VERIFY_SUMMARY=""
VERIFY_FEEDBACK=""
VERIFY_COMMANDS=""
COMMIT_SUBJECT=""
COMMIT_BODY=""
VERIFY_SCHEMA=""
COMMIT_SCHEMA=""

ISSUE_TITLE="$(issue_title "$ISSUE_PATH")"
write_schemas

echo "Selected issue: $ISSUE_PATH"
run_initial_implementer "$ISSUE_PATH" "$ISSUE_TITLE"

for cycle in $(seq 1 "$MAX_CYCLES"); do
  echo "Verification cycle $cycle/$MAX_CYCLES"
  run_verifier "$ISSUE_PATH" "$ISSUE_TITLE" "$cycle"

  if [[ "$VERIFY_STATUS" == "pass" ]]; then
    set_issue_status "$ISSUE_PATH" complete
    append_issue_comment "$ISSUE_PATH" "AFK completed" "$VERIFY_SUMMARY"
    generate_commit_message "$ISSUE_PATH" "$ISSUE_TITLE"
    git add -A
    commit_changes
    echo "Completed issue: $ISSUE_PATH"
    exit 0
  fi

  echo "Verifier failed cycle $cycle/$MAX_CYCLES"

  if ((cycle == MAX_CYCLES)); then
    set_issue_status "$ISSUE_PATH" blocked
    append_issue_comment "$ISSUE_PATH" "AFK blocked after ${MAX_CYCLES} cycles" "$VERIFY_FEEDBACK"
    echo "Blocked issue after $MAX_CYCLES cycles: $ISSUE_PATH" >&2
    exit 1
  fi

  resume_implementer "$ISSUE_PATH" "$cycle" "$VERIFY_FEEDBACK"
done
