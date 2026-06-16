#!/usr/bin/env bash
set -euo pipefail

MAX_CYCLES=5
IMPLEMENTER_MODEL="opencode-go/minimax-m3"
QUALITY_MODEL="opencode-go/minimax-m3"
#VERIFIER_MODEL="openai/gpt-5.5"
VERIFIER_MODEL="opencode-go/minimax-m3"
SBX_PROFILE="${SBX_PROFILE:-opencode-tgreddit}"

usage() {
  echo "Usage: scripts/run_opencode_afk.sh [--all | --prd <feature-slug|path-to-PRD.md>]" >&2
}

parse_args() {
  REQUESTED_PRD=""
  RUN_ALL=0

  while (($# > 0)); do
    case "$1" in
      --all)
        RUN_ALL=1
        shift
        ;;
      --prd)
        if (($# < 2)) || [[ -z "$2" ]]; then
          usage
          exit 2
        fi
        REQUESTED_PRD="$2"
        shift 2
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        usage
        exit 2
        ;;
    esac
  done

  if ((RUN_ALL)) && [[ -n "$REQUESTED_PRD" ]]; then
    echo "--all cannot be combined with --prd." >&2
    usage
    exit 2
  fi
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
  local last_tool_summary=""
  local text
  local clean_line
  local display_line
  local json_line
  local summary

  while IFS= read -r line; do
    printf '%s\n' "$line" >> "$out"
    clean_line="${line//$'\r'/}"
    display_line="$clean_line"
    json_line="$clean_line"

    if ! jq -e . >/dev/null 2>&1 <<< "$json_line"; then
      if [[ "$clean_line" == *\{* ]]; then
        json_line="{${clean_line#*\{}"
      elif [[ "$clean_line" == *\[* ]]; then
        json_line="[${clean_line#*\[}"
      fi
    fi

    if ! jq -e . >/dev/null 2>&1 <<< "$json_line"; then
      if [[ "$clean_line" != *\{* && "$clean_line" != *\[* ]]; then
        display_line="$(strip_terminal_controls "$display_line")"
        if [[ "$display_line" =~ [^[:space:]] ]]; then
          print_progress_line "$display_line"
        fi
      fi
      last_tool_summary=""
      continue
    fi

    while IFS= read -r text; do
      if [[ -z "$text" ]]; then
        continue
      fi

      print_progress_line "$text"
    done < <(jq -r '
      [
        .part?,
        .properties?.part?
      ]
      | .[]?
      | select(type == "object")
      | select(.type == "text" or .type == "reasoning")
      | .text?
      | select(type == "string")
      | gsub("\r"; "")
      | gsub("\u001b\\[[0-9;?]*[ -/]*[@-~]"; "")
      | split("<system-reminder>")[0]
      | split("\n")[]
      | sub("^[[:space:]]+"; "")
      | select(test("\\S"))
    ' <<< "$json_line")

    while IFS= read -r summary; do
      if [[ -z "$summary" || "$summary" == "$last_tool_summary" ]]; then
        continue
      fi

      print_progress_line "$summary"
      last_tool_summary="$summary"
    done < <(jq -r '
      [
        .part?,
        .properties?.part?
      ]
      | .[]?
      | select(type == "object")
      | select(.type == "tool")
      | select(.tool | type == "string")
      | .tool as $tool
      | (.state? // {}) as $state
      | ($state.status? // "") as $status
      | (
        $state.title? //
          $state.input?.description? //
          $state.input?.filePath? //
          $state.input?.command? //
          (if $status == "error" then $state.error? else empty end) //
          ""
        )
        | gsub("\r"; "")
        | gsub("\u001b\\[[0-9;?]*[ -/]*[@-~]"; "")
        | split("<system-reminder>")[0]
        | split("\n")[0]
        | sub("^[[:space:]]+"; "")
        | .[0:140] as $title
      | (
          if $status == "completed" then "Finished tool"
          elif $status == "error" then "Tool failed"
          elif $status == "pending" or $status == "running" then "Running tool"
          else "Tool"
          end
        ) as $prefix
      | $prefix + ": " + $tool +
        (if ($title | type == "string" and length > 0) then " - " + $title else "" end)
    ' <<< "$json_line")
  done
}

strip_terminal_controls() {
  printf '%s' "$1" | sed -E $'s/\x1B\[[0-9;?]*[ -\/]*[@-~]//g'
}

print_progress_line() {
  printf '\r\033[K%s\n' "$1"
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

    opencode plugin @tarquinen/opencode-dcp@latest --global
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

first_runnable_issue_in_feature() {
  local feature_dir="$1"

  find "$feature_dir/issues" -name '*.md' -type f 2>/dev/null \
    | sort \
    | while IFS= read -r issue; do
      if is_runnable_issue "$issue"; then
        printf '%s\n' "$issue"
        return 0
      fi
    done
}

first_runnable_issue_across_prds() {
  local -a prds
  local prd
  local issue

  if [[ ! -d .scratch ]]; then
    return 0
  fi

  mapfile -t prds < <(find .scratch -mindepth 2 -maxdepth 2 -name 'PRD.md' -type f 2>/dev/null | sort)
  for prd in "${prds[@]}"; do
    issue="$(first_runnable_issue_in_feature "$(feature_dir_for_prd "$prd")" || true)"
    if [[ -n "$issue" ]]; then
      printf '%s\n' "$issue"
      return 0
    fi
  done
}

feature_dir_for_issue() {
  local issue="$1"

  dirname "$(dirname "$issue")"
}

feature_dir_for_prd() {
  local prd="$1"

  dirname "$prd"
}

normalize_prd_path() {
  local prd="$1"

  case "$prd" in
    ./*)
      printf '%s\n' "${prd#./}"
      ;;
    *)
      printf '%s\n' "$prd"
      ;;
  esac
}

resolve_prd() {
  local requested="$1"
  local prd
  local feature_dir

  if [[ "$requested" == */* ]]; then
    prd="$(normalize_prd_path "$requested")"
  else
    prd=".scratch/$requested/PRD.md"
  fi

  if [[ ! -f "$prd" ]]; then
    echo "PRD not found: $requested" >&2
    exit 1
  fi
  if [[ "$(basename "$prd")" != "PRD.md" ]]; then
    echo "Selected path is not a PRD.md file: $prd" >&2
    exit 1
  fi
  feature_dir="$(feature_dir_for_prd "$prd")"
  if [[ "$(dirname "$feature_dir")" != ".scratch" ]]; then
    echo "PRD must be under .scratch/<feature>/PRD.md: $prd" >&2
    exit 1
  fi

  printf '%s\n' "$prd"
}

select_prd() {
  local -a prds
  local prd

  if [[ ! -d .scratch ]]; then
    return 0
  fi

  mapfile -t prds < <(find .scratch -mindepth 2 -maxdepth 2 -name 'PRD.md' -type f 2>/dev/null | sort)
  for prd in "${prds[@]}"; do
    if [[ -n "$(first_runnable_issue_in_feature "$(feature_dir_for_prd "$prd")" || true)" ]]; then
      printf '%s\n' "$prd"
      return 0
    fi
  done
}

start_issue() {
  local issue="$1"

  ISSUE_PATH="$issue"
  ISSUE_TITLE="$(issue_title "$ISSUE_PATH")"
  IMPLEMENTER_TITLE="$(implementer_title "$ISSUE_PATH")"
  STATE_PATH="$(state_path_for_issue "$ISSUE_PATH")"
  STATE_PHASE="initial_implement"
  CYCLE=1
  SAVED_FEEDBACK=""
  IMPLEMENTER_SESSION_ID=""
  SESSION_PHASE=""
  SESSION_ID=""
  VERIFY_STATUS=""
  VERIFY_SUMMARY=""
  VERIFY_FEEDBACK=""
  VERIFY_COMMANDS=""
  VERIFY_COMMIT=""
  echo "Selected issue: $ISSUE_PATH"
}

start_next_afkable_issue() {
  local issue

  : > "$SKIP_REASONS"
  issue="$(first_runnable_issue_across_prds || true)"
  if [[ -n "$issue" ]]; then
    start_issue "$issue"
    return 0
  fi

  echo "No AFKable issues found."
  if [[ -s "$SKIP_REASONS" ]]; then
    echo >&2
    while IFS= read -r line; do
      echo "$line" >&2
    done < "$SKIP_REASONS"
  fi
  return 1
}

start_next_issue_in_feature() {
  local feature_dir="$1"
  local issue

  : > "$SKIP_REASONS"
  issue="$(first_runnable_issue_in_feature "$feature_dir" || true)"
  if [[ -n "$issue" ]]; then
    start_issue "$issue"
    return 0
  fi

  echo "No AFKable issues found."
  if [[ -s "$SKIP_REASONS" ]]; then
    echo >&2
    while IFS= read -r line; do
      echo "$line" >&2
    done < "$SKIP_REASONS"
  fi
  return 1
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
  local version

  STATE_PATH="$state"
  ISSUE_PATH="$(jq -r '.issue_path' "$state")"
  ISSUE_TITLE="$(jq -r '.issue_title' "$state")"
  IMPLEMENTER_TITLE="$(implementer_title "$ISSUE_PATH")"
  STATE_PHASE="$(jq -r '.phase' "$state")"
  CYCLE="$(jq -r '.cycle' "$state")"
  SAVED_FEEDBACK="$(jq -r '.feedback // ""' "$state")"
  version="$(jq -r '.version // 0' "$state")"

  if [[ "$version" != "3" ]]; then
    IMPLEMENTER_SESSION_ID=""
    SESSION_PHASE=""
    SESSION_ID=""
    save_state "$STATE_PHASE" "$CYCLE" "$SAVED_FEEDBACK"
    echo "Regenerated AFK state schema: $state"
    return 0
  fi

  IMPLEMENTER_SESSION_ID="$(normalize_session_id "$(jq -r '.implementer_session_id // ""' "$state")")"
  SESSION_PHASE="$(jq -r '.session_phase // ""' "$state")"
  SESSION_ID="$(normalize_session_id "$(jq -r '.session_id // ""' "$state")")"
  SAVED_FEEDBACK="$(jq -r '.feedback // ""' "$state")"
}

normalize_session_id() {
  case "$1" in
    ""|null|'""')
      printf ''
      ;;
    *)
      printf '%s' "$1"
      ;;
  esac
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
    --arg implementer_session_id "$IMPLEMENTER_SESSION_ID" \
    --arg session_phase "$SESSION_PHASE" \
    --arg session_id "$SESSION_ID" \
    --arg feedback "$feedback" \
    --argjson cycle "$cycle" \
    '{
      version: 3,
      harness: "opencode",
      issue_path: $issue_path,
      issue_title: $issue_title,
      phase: $phase,
      cycle: $cycle,
      implementer_session_id: $implementer_session_id,
      session_phase: $session_phase,
      session_id: $session_id,
      feedback: $feedback
    }' > "$tmp_file"
  mv "$tmp_file" "$STATE_PATH"
}

clear_transient_session() {
  SESSION_PHASE=""
  SESSION_ID=""
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

  if [[ -n "$IMPLEMENTER_SESSION_ID" || -z "${IMPLEMENTER_TITLE:-}" ]]; then
    return 0
  fi

  if ! sbx exec -d "$SBX_PROFILE" bash -lc 'cd /home/dima/Desktop/tgreddit && opencode session list --format json --max-count 50 | jq -c .' > "$sessions"; then
    return 1
  fi

  recovered="$(jq -Rrc --arg dir "$PWD" --arg title "$IMPLEMENTER_TITLE" '
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
  recovered="$(normalize_session_id "$recovered")"

  if [[ -n "$recovered" ]]; then
    IMPLEMENTER_SESSION_ID="$recovered"
    echo "Recovered implementer session: $IMPLEMENTER_SESSION_ID"
  fi
}

capture_session_id() {
  local out="$1"
  local target="$2"
  local captured

  if [[ ! -f "$out" ]]; then
    return 0
  fi

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
  captured="$(normalize_session_id "$captured")"
  if [[ -n "$captured" ]]; then
    printf -v "$target" '%s' "$captured"
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
    (.status == "pass" or .status == "fail" or .status == "needs-info" or .status == "blocked") and
    (.summary | type == "string") and
    (.feedback | type == "string") and
    (.commands_run | type == "array")
  ' "$last" >/dev/null
}

extract_text_parts() {
  local out="$1"

  if [[ ! -f "$out" ]]; then
    return 0
  fi

  json_lines "$out" | jq -rs '
    [
      .[]?
      | select(type == "object")
      | .part?, .properties?.part?
      | select(type == "object" and (.type == "text" or .type == "reasoning"))
      | .text?
      | select(type == "string")
    ]
    | add // ""
  '
}

extract_agent_status() {
  local out="$1"
  local text
  local last_line
  local status

  text="$(extract_text_parts "$out")"
  last_line="$(printf '%s' "$text" | awk 'NF {p=1} p' | tail -n 1)"
  status="$(jq -re '.status // empty' <<< "$last_line" 2>/dev/null || true)"
  printf '%s\n' "$status"
}

extract_agent_reason() {
  local out="$1"
  local text
  local trimmed
  local last_line

  text="$(extract_text_parts "$out")"
  trimmed="$(printf '%s' "$text" | awk 'NF {p=1} p')"
  last_line="$(printf '%s' "$trimmed" | tail -n 1)"

  if jq -e '.status' <<< "$last_line" >/dev/null 2>&1; then
    printf '%s' "$trimmed" | sed '$d'
  else
    printf '%s' "$text"
  fi
}

detect_agent_status() {
  local out="$1"
  local opencode_exit="${2:-0}"
  local status

  status="$(extract_agent_status "$out")"
  case "$status" in
    pass|fail|needs-info|blocked)
      printf '%s\n' "$status"
      ;;
    *)
      if ((opencode_exit != 0)); then
        printf 'fail\n'
      else
        printf 'pass\n'
      fi
      ;;
  esac
}

reset_loop_action() {
  LOOP_ACTION=""
  LOOP_EXIT_STATUS=0
}

set_loop_continue() {
  LOOP_ACTION="continue"
}

set_loop_exit() {
  LOOP_ACTION="exit"
  LOOP_EXIT_STATUS="${1:-1}"
}

handle_terminal_blocker() {
  local status="$1"
  local reason="$2"
  local label

  case "$status" in
    needs-info)
      label="needs-info"
      ;;
    blocked)
      label="blocked"
      ;;
    *)
      return 1
      ;;
  esac

  set_issue_status "$ISSUE_PATH" "$label"
  append_issue_comment "$ISSUE_PATH" "AFK $label" "$reason"
  clear_state
  echo "Issue $label: $ISSUE_PATH"

  if ((RUN_ALL)); then
    if ! start_next_afkable_issue; then
      exit 0
    fi
    set_loop_continue
  else
    exit 0
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

run_initial_implementer() {
  local issue="$1"
  local title="$2"
  local out="$TMPDIR/implement-initial.jsonl"
  local opencode_exit=0
  local status
  local reason

  save_state "initial_implement" 1 ""
  set +e
  opencode_afk_run \
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
4. End your final message with exactly one JSON line: \`{"status":"pass"}\` on success, \`{"status":"needs-info"}\` when waiting on context/credentials, or \`{"status":"blocked"}\` when unresolvable/unsafe. Put any explanation before the JSON line.

Rules:
- Do not implement out-of-scope features.
- Do not change unrelated behavior.
- Leave the worktree ready for code-quality review.
PROMPT
)" 2>&1 | show_opencode_progress "$out"
  opencode_exit=${PIPESTATUS[0]}
  set -e

  capture_session_id "$out" IMPLEMENTER_SESSION_ID
  if [[ -z "$IMPLEMENTER_SESSION_ID" || "$IMPLEMENTER_SESSION_ID" == "null" ]]; then
    recover_implementer_session_id || true
  fi
  if [[ -z "$IMPLEMENTER_SESSION_ID" || "$IMPLEMENTER_SESSION_ID" == "null" ]]; then
    echo "Could not capture implementer session id." >&2
    exit 1
  fi

  status="$(detect_agent_status "$out" "$opencode_exit")"
  case "$status" in
    pass)
      clear_transient_session
      save_state "code_quality" 1 ""
      ;;
    needs-info|blocked)
      reason="$(extract_agent_reason "$out")"
      handle_terminal_blocker "$status" "$reason"
      ;;
    *)
      save_state "initial_implement" 1 ""
      set_loop_exit 1
      ;;
  esac
}

continue_implementer() {
  local issue="$1"
  local cycle="$2"
  local next_cycle="$3"
  local out="$TMPDIR/implement-continue-${cycle}.jsonl"
  local opencode_exit=0
  local status
  local reason

  set +e
  opencode_afk_resume \
    afk-implementer \
    "$IMPLEMENTER_MODEL" \
    "$IMPLEMENTER_SESSION_ID" \
    "$(cat <<PROMPT
Continue the AFK implementation for local issue:

${issue}

Continue from the current repository state. Do not commit. Leave the worktree ready for code-quality review.

End your final message with exactly one JSON line: \`{"status":"pass"}\`, \`{"status":"needs-info"}\`, or \`{"status":"blocked"}\`. Put any explanation before the JSON line.
PROMPT
)" 2>&1 | show_opencode_progress "$out"
  opencode_exit=${PIPESTATUS[0]}
  set -e

  status="$(detect_agent_status "$out" "$opencode_exit")"
  case "$status" in
    pass)
      clear_transient_session
      save_state "code_quality" "$next_cycle" ""
      ;;
    needs-info|blocked)
      reason="$(extract_agent_reason "$out")"
      handle_terminal_blocker "$status" "$reason"
      ;;
    *)
      save_state "$STATE_PHASE" "$cycle" "$SAVED_FEEDBACK"
      set_loop_exit 1
      ;;
  esac
}

resume_implementer() {
  local issue="$1"
  local cycle="$2"
  local feedback="$3"
  local out="$TMPDIR/implement-cycle-${cycle}.jsonl"
  local opencode_exit=0
  local status
  local reason

  if [[ -z "$IMPLEMENTER_SESSION_ID" ]]; then
    recover_implementer_session_id || true
  fi
  if [[ -z "$IMPLEMENTER_SESSION_ID" ]]; then
    echo "Could not resume implementer: missing implementer session id." >&2
    echo "Remove $STATE_PATH to restart the issue from scratch." >&2
    exit 1
  fi

  save_state "resume_implement" "$cycle" "$feedback"
  set +e
  opencode_afk_resume \
    afk-implementer \
    "$IMPLEMENTER_MODEL" \
    "$IMPLEMENTER_SESSION_ID" \
    "$(cat <<PROMPT
Verifier failed cycle ${cycle} for local issue:

${issue}

Address the verifier feedback below, then leave the worktree ready for code-quality review again.

Do not commit.
Do not change out-of-scope behavior.

End your final message with exactly one JSON line: \`{"status":"pass"}\`, \`{"status":"needs-info"}\`, or \`{"status":"blocked"}\`. Put any explanation before the JSON line.

Verifier feedback:

${feedback}
PROMPT
)" 2>&1 | show_opencode_progress "$out"
  opencode_exit=${PIPESTATUS[0]}
  set -e

  status="$(detect_agent_status "$out" "$opencode_exit")"
  case "$status" in
    pass)
      clear_transient_session
      save_state "code_quality" "$((cycle + 1))" ""
      ;;
    needs-info|blocked)
      reason="$(extract_agent_reason "$out")"
      handle_terminal_blocker "$status" "$reason"
      ;;
    *)
      save_state "resume_implement" "$cycle" "$feedback"
      set_loop_exit 1
      ;;
  esac
}

run_code_quality() {
  local issue="$1"
  local title="$2"
  local cycle="$3"
  local out="$TMPDIR/code-quality-cycle-${cycle}.jsonl"
  local prompt
  local opencode_exit=0
  local status
  local reason

  prompt="$(cat <<PROMPT
You are reviewing and improving the current AFK implementation for local issue:

${issue}

Issue title: ${title}

Your job:
1. Read the issue file, relevant docs, and current diff.
2. Improve code quality without expanding scope.
3. Pay special attention to cheap-model artifacts: overengineering, duplicated logic, bad names, brittle tests, missed edge cases, poor Rust idioms, and accidental unrelated changes.
4. Run relevant validation commands when feasible.
5. Do not commit.

End your final message with exactly one JSON line: \`{"status":"pass"}\`, \`{"status":"needs-info"}\`, or \`{"status":"blocked"}\`. Put any explanation before the JSON line.

Leave the worktree ready for final verification.
PROMPT
)"

  if [[ "$SESSION_PHASE" != "code_quality" ]]; then
    clear_transient_session
  fi
  if [[ -z "$SESSION_ID" ]]; then
    SESSION_PHASE="code_quality"
  fi
  save_state "code_quality" "$cycle" ""

  set +e
  if [[ -n "$SESSION_ID" ]]; then
    echo "Resumed code-quality session: $SESSION_ID"
    opencode_afk_resume \
      afk-code-quality \
      "$QUALITY_MODEL" \
      "$SESSION_ID" \
      "$prompt" 2>&1 | show_opencode_progress "$out"
  else
    opencode_afk_run \
      afk-code-quality \
      "$QUALITY_MODEL" \
      "" \
      "$prompt" 2>&1 | show_opencode_progress "$out"
  fi
  opencode_exit=${PIPESTATUS[0]}
  set -e

  capture_session_id "$out" SESSION_ID

  status="$(detect_agent_status "$out" "$opencode_exit")"
  case "$status" in
    pass)
      clear_transient_session
      save_state "verify" "$cycle" ""
      ;;
    needs-info|blocked)
      reason="$(extract_agent_reason "$out")"
      handle_terminal_blocker "$status" "$reason"
      ;;
    *)
      SESSION_PHASE="code_quality"
      save_state "code_quality" "$cycle" ""
      set_loop_exit 1
      ;;
  esac
}

run_verifier() {
  local issue="$1"
  local title="$2"
  local cycle="$3"
  local out="$TMPDIR/verify-cycle-${cycle}.jsonl"
  local last="$TMPDIR/verify-cycle-${cycle}.json"
  local prompt

  prompt="$(cat <<PROMPT
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

If you cannot proceed because of missing credentials, ambiguous spec, or unsafe conditions:
1. Do not commit.
2. Return valid JSON with status "needs-info" or "blocked" and the reason in feedback.

Final response requirements:
- JSON object only.
- No markdown.
- No code fences.
- Include keys: status, summary, feedback, commands_run, commit.
PROMPT
)"

  if [[ "$SESSION_PHASE" != "verify" ]]; then
    clear_transient_session
  fi
  if [[ -z "$SESSION_ID" ]]; then
    SESSION_PHASE="verify"
  fi
  save_state "verify" "$cycle" ""
  if [[ -n "$SESSION_ID" ]]; then
    echo "Resumed verifier session: $SESSION_ID"
    if ! opencode_afk_resume \
      afk-verifier \
      "$VERIFIER_MODEL" \
      "$SESSION_ID" \
      "$prompt" 2>&1 | show_opencode_progress "$out"; then
      capture_session_id "$out" SESSION_ID
      SESSION_PHASE="verify"
      save_state "verify" "$cycle" ""
      return 1
    fi
  else
    if ! opencode_afk_run \
      afk-verifier \
      "$VERIFIER_MODEL" \
      "" \
      "$prompt" 2>&1 | show_opencode_progress "$out"; then
      capture_session_id "$out" SESSION_ID
      SESSION_PHASE="verify"
      save_state "verify" "$cycle" ""
      return 1
    fi
  fi

  capture_session_id "$out" SESSION_ID
  SESSION_PHASE="verify"
  save_state "verify" "$cycle" ""

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

  case "$VERIFY_STATUS" in
    pass|fail)
      ;;
    needs-info|blocked)
      handle_terminal_blocker "$VERIFY_STATUS" "$VERIFY_FEEDBACK"
      ;;
    *)
      echo "Verifier returned invalid status: $VERIFY_STATUS" >&2
      exit 1
      ;;
  esac
}

# Allow the harness to be sourced by test scripts without executing the main loop.
if [[ -n "${AFK_HARNESS_TEST:-}" ]]; then
  return 0
fi

REQUESTED_PRD=""
parse_args "$@"

require_cmd jq

TMPDIR="$(mktemp -d)"
cleanup() {
  local status=$?
  local recovered=0

  if ((status != 0)) && [[ -n "${STATE_PATH:-}" && -f "${STATE_PATH:-}" ]]; then
    case "${STATE_PHASE:-}" in
      initial_implement|resume_implement)
        if [[ -z "${IMPLEMENTER_SESSION_ID:-}" ]]; then
          recover_implementer_session_id || true
          if [[ -n "${IMPLEMENTER_SESSION_ID:-}" ]]; then
            recovered=1
          fi
        fi
        ;;
      code_quality)
        if [[ "${SESSION_PHASE:-}" != "code_quality" || -z "${SESSION_ID:-}" ]]; then
          if [[ -f "$TMPDIR/code-quality-cycle-${CYCLE}.jsonl" ]]; then
            capture_session_id "$TMPDIR/code-quality-cycle-${CYCLE}.jsonl" SESSION_ID
            if [[ -n "${SESSION_ID:-}" ]]; then
              SESSION_PHASE="code_quality"
              recovered=1
            fi
          fi
        fi
        ;;
      verify)
        if [[ "${SESSION_PHASE:-}" != "verify" || -z "${SESSION_ID:-}" ]]; then
          if [[ -f "$TMPDIR/verify-cycle-${CYCLE}.jsonl" ]]; then
            capture_session_id "$TMPDIR/verify-cycle-${CYCLE}.jsonl" SESSION_ID
            if [[ -n "${SESSION_ID:-}" ]]; then
              SESSION_PHASE="verify"
              recovered=1
            fi
          fi
        fi
        ;;
    esac

    if ((recovered)); then
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
IMPLEMENTER_SESSION_ID=""
SESSION_PHASE=""
SESSION_ID=""
IMPLEMENTER_TITLE=""
VERIFY_STATUS=""
VERIFY_SUMMARY=""
VERIFY_FEEDBACK=""
VERIFY_COMMANDS=""
VERIFY_COMMIT=""
SELECTED_PRD=""
SELECTED_FEATURE_DIR=""
LOOP_ACTION=""
LOOP_EXIT_STATUS=0

EXISTING_STATE="$(active_state_file)"
if [[ -n "$EXISTING_STATE" ]]; then
  load_state "$EXISTING_STATE"
  if [[ -n "$REQUESTED_PRD" ]]; then
    SELECTED_PRD="$(resolve_prd "$REQUESTED_PRD")"
    SELECTED_FEATURE_DIR="$(feature_dir_for_prd "$SELECTED_PRD")"
    if [[ "$(feature_dir_for_issue "$ISSUE_PATH")" != "$SELECTED_FEATURE_DIR" ]]; then
      echo "Active AFK state is for $ISSUE_PATH, not selected PRD $SELECTED_PRD." >&2
      echo "Remove $STATE_PATH to start a different PRD." >&2
      exit 1
    fi
  fi
  echo "Resuming issue: $ISSUE_PATH"
  echo "AFK state: $STATE_PATH"
else
  if ((RUN_ALL)); then
    if ! start_next_afkable_issue; then
      exit 0
    fi
  elif [[ -n "$REQUESTED_PRD" ]]; then
    SELECTED_PRD="$(resolve_prd "$REQUESTED_PRD")"
    SELECTED_FEATURE_DIR="$(feature_dir_for_prd "$SELECTED_PRD")"
    echo "Selected PRD: $SELECTED_PRD"
    if ! start_next_issue_in_feature "$SELECTED_FEATURE_DIR"; then
      exit 0
    fi
  else
    SELECTED_PRD="$(select_prd)"
    if [[ -z "$SELECTED_PRD" ]]; then
      echo "No AFKable issues found."
      exit 0
    fi
    SELECTED_FEATURE_DIR="$(feature_dir_for_prd "$SELECTED_PRD")"
    echo "Selected PRD: $SELECTED_PRD"
    if ! start_next_issue_in_feature "$SELECTED_FEATURE_DIR"; then
      exit 0
    fi
  fi
fi

require_cmd sbx
require_cmd git
run_preflight

while true; do
  reset_loop_action

  case "$STATE_PHASE" in
    initial_implement)
      if [[ -z "$IMPLEMENTER_SESSION_ID" ]]; then
        recover_implementer_session_id || true
      fi
      if [[ -n "$IMPLEMENTER_SESSION_ID" ]]; then
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
        if ((RUN_ALL)); then
          if ! start_next_afkable_issue; then
            exit 0
          fi
          set_loop_continue
        else
          exit 0
        fi
      else
        echo "Verifier failed cycle $CYCLE/$MAX_CYCLES"

        if ((CYCLE == MAX_CYCLES)); then
          set_issue_status "$ISSUE_PATH" blocked
          append_issue_comment "$ISSUE_PATH" "AFK blocked after ${MAX_CYCLES} cycles" "$VERIFY_FEEDBACK"
          clear_state
          echo "Blocked issue after $MAX_CYCLES cycles: $ISSUE_PATH" >&2
          if ((RUN_ALL)); then
            if ! start_next_afkable_issue; then
              exit 1
            fi
            set_loop_continue
          else
            exit 1
          fi
        fi

        clear_transient_session
        resume_implementer "$ISSUE_PATH" "$CYCLE" "$VERIFY_FEEDBACK"
      fi
      ;;
    *)
      echo "Unknown AFK state phase: $STATE_PHASE" >&2
      exit 1
      ;;
  esac

  case "$LOOP_ACTION" in
    continue)
      ;;
    exit)
      exit "$LOOP_EXIT_STATUS"
      ;;
    *)
      load_state "$STATE_PATH"
      ;;
  esac
done
