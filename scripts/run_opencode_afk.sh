#!/usr/bin/env bash
set -euo pipefail

MAX_CYCLES=5
IMPLEMENTER_MODEL="opencode-go/minimax-m3"
QUALITY_MODEL="opencode-go/minimax-m3"
VERIFIER_MODEL="openai/gpt-5.5"
SBX_PROFILE="${SBX_PROFILE:-opencode-tgreddit}"

usage() {
  echo "Usage: scripts/run_opencode_afk.sh" >&2
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "$1 is required." >&2
    exit 1
  fi
}

opencode_afk_run() {
  local agent="$1"
  local model="$2"
  local title="$3"
  shift 3

  local title_args=()
  if [[ -n "$title" ]]; then
    title_args=(--title "$title")
  fi

  sbx run "$SBX_PROFILE" -- run \
    --format json \
    --dir "$PWD" \
    --agent "$agent" \
    --model "$model" \
    "${title_args[@]}" \
    --dangerously-skip-permissions \
    "$@"
}

opencode_afk_resume() {
  local agent="$1"
  local model="$2"
  local session_id="$3"
  shift 3

  sbx run "$SBX_PROFILE" -- run \
    --format json \
    --dir "$PWD" \
    --session "$session_id" \
    --agent "$agent" \
    --model "$model" \
    --dangerously-skip-permissions \
    "$@"
}

show_opencode_progress() {
  local out="$1"

  while IFS= read -r line; do
    printf '%s\n' "$line" >> "$out"
    if ! jq -e . >/dev/null 2>&1 <<< "$line"; then
      printf '%s\n' "$line"
    fi
  done
}

json_lines() {
  local out="$1"

  jq -Rc 'fromjson?' "$out"
}

run_preflight() {
  echo "Running AFK sandbox preflight"

  sbx run "$SBX_PROFILE" -- --version >/dev/null
  sbx exec -d "$SBX_PROFILE" bash -lc '
    set -euo pipefail

    cd /home/dima/Desktop/tgreddit

    need() {
      if ! command -v "$1" >/dev/null; then
        echo "$1 is required in the AFK sandbox." >&2
        exit 1
      fi
    }

    need git
    need jq
    need cargo
    cargo --version >/dev/null
    cargo fmt --version >/dev/null
    cargo clippy --version >/dev/null
    need yt-dlp
    yt-dlp --version >/dev/null
    need ffmpeg
    ffmpeg -version >/dev/null
    test -f tgreddit.toml
    test -f telegram-e2e.toml
  '
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

state_path_for_issue() {
  local issue="$1"
  local feature_dir

  feature_dir="$(dirname "$(dirname "$issue")")"
  printf '%s/.afk-state.json\n' "$feature_dir"
}

active_state_file() {
  local states

  mapfile -t states < <(find .scratch -path '*/.afk-state.json' -type f 2>/dev/null | sort)
  if ((${#states[@]} > 1)); then
    echo "Multiple AFK state files found; remove all but one before resuming:" >&2
    printf '%s\n' "${states[@]}" >&2
    exit 1
  fi
  if ((${#states[@]} == 1)); then
    printf '%s\n' "${states[0]}"
  fi
}

load_state() {
  local state="$1"

  ISSUE_PATH="$(jq -r '.issue_path' "$state")"
  ISSUE_TITLE="$(jq -r '.issue_title' "$state")"
  IMPLEMENTER_TITLE="$(implementer_title "$ISSUE_PATH")"
  STATE_PHASE="$(jq -r '.phase' "$state")"
  CYCLE="$(jq -r '.cycle' "$state")"
  SESSION_ID="$(jq -r '.session_id // ""' "$state")"
  SAVED_FEEDBACK="$(jq -r '.feedback // ""' "$state")"
  STATE_PATH="$state"
}

save_state() {
  local phase="$1"
  local cycle="$2"
  local feedback="${3:-}"
  local tmp_file="$TMPDIR/afk-state.json"

  jq -n \
    --arg issue_path "$ISSUE_PATH" \
    --arg issue_title "$ISSUE_TITLE" \
    --arg phase "$phase" \
    --arg session_id "$SESSION_ID" \
    --arg feedback "$feedback" \
    --argjson cycle "$cycle" \
    '{
      version: 2,
      harness: "opencode",
      issue_path: $issue_path,
      issue_title: $issue_title,
      phase: $phase,
      cycle: $cycle,
      session_id: $session_id,
      feedback: $feedback
    }' > "$tmp_file"
  mv "$tmp_file" "$STATE_PATH"
}

clear_state() {
  if [[ -n "${STATE_PATH:-}" && -f "$STATE_PATH" ]]; then
    rm -f "$STATE_PATH"
  fi
}

implementer_title() {
  local issue="$1"

  printf 'AFK implementer: %s\n' "$issue"
}

recover_implementer_session_id() {
  local sessions="$TMPDIR/opencode-sessions.jsonl"
  local recovered

  if [[ -n "$SESSION_ID" || -z "${IMPLEMENTER_TITLE:-}" ]]; then
    return 0
  fi

  if ! sbx exec -d "$SBX_PROFILE" bash -lc 'cd /home/dima/Desktop/tgreddit && opencode session list --format json --max-count 50 | jq -c .' > "$sessions"; then
    return 1
  fi

  recovered="$(jq -Rc --arg dir "$PWD" --arg title "$IMPLEMENTER_TITLE" '
    fromjson? |
    select(type == "array") |
    [
      .[] |
      select(.directory == $dir and .title == $title)
    ] |
    sort_by(.updated) |
    last |
    .id // ""
  ' "$sessions" | sed -n '1p')"

  if [[ -n "$recovered" && "$recovered" != "null" ]]; then
    SESSION_ID="$recovered"
    echo "Recovered implementer session: $SESSION_ID"
  fi
}

capture_session_id() {
  local out="$1"
  local captured

  captured="$(json_lines "$out" | jq -r '
    select(type == "object") |
    [
      .sessionID?,
      .session_id?,
      .sessionId?,
      .session?.id?,
      .properties?.sessionID?,
      .properties?.session_id?,
      .properties?.sessionId?
    ] |
    .[]? |
    select(type == "string" and length > 0)
  ' | sed -n '1p')"
  if [[ -n "$captured" && "$captured" != "null" ]]; then
    SESSION_ID="$captured"
  fi
}

extract_verifier_result() {
  local out="$1"
  local last="$2"
  local result

  result="$(json_lines "$out" | jq -rs '
    [
      .. |
      strings |
      select(contains("\"status\"") and contains("\"commands_run\""))
    ] |
    last // ""
  ')"

  if [[ -z "$result" || "$result" == '""' ]]; then
    return 1
  fi

  printf '%s\n' "$result" | jq -r . > "$last"
  jq -e '
    type == "object" and
    (.status == "pass" or .status == "fail") and
    (.summary | type == "string") and
    (.feedback | type == "string") and
    (.commands_run | type == "array")
  ' "$last" >/dev/null
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

run_initial_implementer() {
  local issue="$1"
  local title="$2"
  local out="$TMPDIR/implement-initial.jsonl"

  save_state "initial_implement" 1 ""
  if ! opencode_afk_run \
    afk-implementer \
    "$IMPLEMENTER_MODEL" \
    "$IMPLEMENTER_TITLE" \
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
- Leave the worktree ready for code-quality review.
PROMPT
)" | show_opencode_progress "$out"; then
    capture_session_id "$out"
    if [[ -z "$SESSION_ID" || "$SESSION_ID" == "null" ]]; then
      recover_implementer_session_id || true
    fi
    save_state "initial_implement" 1 ""
    return 1
  fi

  capture_session_id "$out"
  if [[ -z "$SESSION_ID" || "$SESSION_ID" == "null" ]]; then
    recover_implementer_session_id || true
  fi
  if [[ -z "$SESSION_ID" || "$SESSION_ID" == "null" ]]; then
    echo "Could not capture implementer session id." >&2
    exit 1
  fi
  save_state "code_quality" 1 ""
}

continue_implementer() {
  local issue="$1"
  local cycle="$2"
  local next_cycle="$3"
  local out="$TMPDIR/implement-continue-${cycle}.jsonl"

  if ! opencode_afk_resume \
    afk-implementer \
    "$IMPLEMENTER_MODEL" \
    "$SESSION_ID" \
    "$(cat <<PROMPT
Continue the AFK implementation for local issue:

${issue}

Continue from the current repository state. Do not commit. Leave the worktree ready for code-quality review.
PROMPT
)" | show_opencode_progress "$out"; then
    save_state "$STATE_PHASE" "$cycle" "$SAVED_FEEDBACK"
    return 1
  fi

  save_state "code_quality" "$next_cycle" ""
}

resume_implementer() {
  local issue="$1"
  local cycle="$2"
  local feedback="$3"
  local out="$TMPDIR/implement-cycle-${cycle}.jsonl"

  save_state "resume_implement" "$cycle" "$feedback"
  if ! opencode_afk_resume \
    afk-implementer \
    "$IMPLEMENTER_MODEL" \
    "$SESSION_ID" \
    "$(cat <<PROMPT
Verifier failed cycle ${cycle} for local issue:

${issue}

Address the verifier feedback below, then leave the worktree ready for code-quality review again.

Do not commit.
Do not change out-of-scope behavior.

Verifier feedback:

${feedback}
PROMPT
)" | show_opencode_progress "$out"; then
    save_state "resume_implement" "$cycle" "$feedback"
    return 1
  fi

  save_state "code_quality" "$((cycle + 1))" ""
}

run_code_quality() {
  local issue="$1"
  local title="$2"
  local cycle="$3"
  local out="$TMPDIR/code-quality-cycle-${cycle}.jsonl"

  save_state "code_quality" "$cycle" ""
  if ! opencode_afk_run \
    afk-code-quality \
    "$QUALITY_MODEL" \
    "" \
    "$(cat <<PROMPT
You are reviewing and improving the current AFK implementation for local issue:

${issue}

Issue title: ${title}

Your job:
1. Read the issue file, relevant docs, and current diff.
2. Improve code quality without expanding scope.
3. Pay special attention to cheap-model artifacts: overengineering, duplicated logic, bad names, brittle tests, missed edge cases, poor Rust idioms, and accidental unrelated changes.
4. Run relevant validation commands when feasible.
5. Do not commit.

Leave the worktree ready for final verification.
PROMPT
)" | show_opencode_progress "$out"; then
    save_state "code_quality" "$cycle" ""
    return 1
  fi

  save_state "verify" "$cycle" ""
}

run_verifier() {
  local issue="$1"
  local title="$2"
  local cycle="$3"
  local out="$TMPDIR/verify-cycle-${cycle}.jsonl"
  local last="$TMPDIR/verify-cycle-${cycle}.json"

  save_state "verify" "$cycle" ""
  if ! opencode_afk_run \
    afk-verifier \
    "$VERIFIER_MODEL" \
    "" \
    "$(cat <<PROMPT
You are an independent verifier for local issue:

${issue}

Issue title: ${title}

Verify the current worktree against the issue and repo rules.

Your job:
1. Read the issue file, relevant docs, and current worktree.
2. Inspect the implementation for correctness, scope, and accidental unrelated changes.
3. Run relevant validation commands yourself.
4. Return pass only if the issue acceptance criteria are met, relevant validation passes, and the change is scoped.

If verification passes:
1. Update the issue Status line to complete.
2. Append an "AFK completed" comment to the issue with a concise summary.
3. Inspect the final diff and stage only intended changes.
4. Run git commit yourself with a concise imperative subject and optional body.
5. Return valid JSON with status "pass" and the commit hash.

If verification fails:
1. Do not commit.
2. Return valid JSON with status "fail" and exact feedback for the implementer.

Final response requirements:
- JSON object only.
- No markdown.
- No code fences.
- Include keys: status, summary, feedback, commands_run, commit.
PROMPT
)" | show_opencode_progress "$out"; then
    save_state "verify" "$cycle" ""
    return 1
  fi

  if ! extract_verifier_result "$out" "$last"; then
    echo "Could not extract verifier JSON result." >&2
    save_state "verify" "$cycle" ""
    return 1
  fi

  VERIFY_STATUS="$(jq -r '.status' "$last")"
  VERIFY_SUMMARY="$(jq -r '.summary' "$last")"
  VERIFY_FEEDBACK="$(jq -r '.feedback' "$last")"
  VERIFY_COMMANDS="$(jq -r '.commands_run | join(", ")' "$last")"
  VERIFY_COMMIT="$(jq -r '.commit // ""' "$last")"

  if [[ "$VERIFY_STATUS" != "pass" && "$VERIFY_STATUS" != "fail" ]]; then
    echo "Verifier returned invalid status: $VERIFY_STATUS" >&2
    exit 1
  fi
}

if (($# != 0)); then
  usage
  exit 2
fi

require_cmd sbx
require_cmd git
require_cmd jq

TMPDIR="$(mktemp -d)"
cleanup() {
  local status=$?

  if ((status != 0)) && [[ -n "${STATE_PATH:-}" && -f "${STATE_PATH:-}" && -z "${SESSION_ID:-}" ]]; then
    recover_implementer_session_id || true
    if [[ -n "${SESSION_ID:-}" ]]; then
      save_state "$STATE_PHASE" "$CYCLE" "$SAVED_FEEDBACK"
    fi
  fi

  rm -rf "$TMPDIR"
  if ((status != 0)) && [[ -n "${STATE_PATH:-}" && -f "${STATE_PATH:-}" ]]; then
    echo "AFK state saved: $STATE_PATH" >&2
  fi
}
trap cleanup EXIT

SKIP_REASONS="$TMPDIR/skipped-issues.txt"

STATE_PATH=""
STATE_PHASE=""
CYCLE=1
SAVED_FEEDBACK=""
SESSION_ID=""
IMPLEMENTER_TITLE=""
VERIFY_STATUS=""
VERIFY_SUMMARY=""
VERIFY_FEEDBACK=""
VERIFY_COMMANDS=""
VERIFY_COMMIT=""

run_preflight

EXISTING_STATE="$(active_state_file)"
if [[ -n "$EXISTING_STATE" ]]; then
  load_state "$EXISTING_STATE"
  echo "Resuming issue: $ISSUE_PATH"
  echo "AFK state: $STATE_PATH"
else
  ISSUE_PATH="$(first_runnable_issue || true)"
  if [[ -z "$ISSUE_PATH" ]]; then
    echo "No runnable ready-for-agent issues found under .scratch/*/issues/*.md" >&2
    if [[ -s "$SKIP_REASONS" ]]; then
      echo >&2
      cat "$SKIP_REASONS" >&2
    fi
    exit 1
  fi

  ISSUE_TITLE="$(issue_title "$ISSUE_PATH")"
  IMPLEMENTER_TITLE="$(implementer_title "$ISSUE_PATH")"
  STATE_PATH="$(state_path_for_issue "$ISSUE_PATH")"
  STATE_PHASE="initial_implement"
  echo "Selected issue: $ISSUE_PATH"
fi

while true; do
  case "$STATE_PHASE" in
    initial_implement)
      if [[ -z "$SESSION_ID" ]]; then
        recover_implementer_session_id || true
      fi
      if [[ -n "$SESSION_ID" ]]; then
        echo "Resuming interrupted implementer"
        continue_implementer "$ISSUE_PATH" "$CYCLE" "$CYCLE"
      else
        run_initial_implementer "$ISSUE_PATH" "$ISSUE_TITLE"
      fi
      ;;
    resume_implement)
      echo "Resuming implementer after verifier feedback"
      resume_implementer "$ISSUE_PATH" "$CYCLE" "$SAVED_FEEDBACK"
      ;;
    code_quality)
      echo "Code-quality cycle $CYCLE/$MAX_CYCLES"
      run_code_quality "$ISSUE_PATH" "$ISSUE_TITLE" "$CYCLE"
      ;;
    verify)
      echo "Verification cycle $CYCLE/$MAX_CYCLES"
      run_verifier "$ISSUE_PATH" "$ISSUE_TITLE" "$CYCLE"

      if [[ "$VERIFY_STATUS" == "pass" ]]; then
        clear_state
        echo "Completed issue: $ISSUE_PATH"
        if [[ -n "$VERIFY_COMMIT" && "$VERIFY_COMMIT" != "null" ]]; then
          echo "Commit: $VERIFY_COMMIT"
        fi
        exit 0
      fi

      echo "Verifier failed cycle $CYCLE/$MAX_CYCLES"

      if ((CYCLE == MAX_CYCLES)); then
        set_issue_status "$ISSUE_PATH" blocked
        append_issue_comment "$ISSUE_PATH" "AFK blocked after ${MAX_CYCLES} cycles" "$VERIFY_FEEDBACK"
        clear_state
        echo "Blocked issue after $MAX_CYCLES cycles: $ISSUE_PATH" >&2
        exit 1
      fi

      resume_implementer "$ISSUE_PATH" "$CYCLE" "$VERIFY_FEEDBACK"
      ;;
    *)
      echo "Unknown AFK state phase: $STATE_PHASE" >&2
      exit 1
      ;;
  esac

  load_state "$STATE_PATH"
done
