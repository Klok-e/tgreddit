# AFK Script Pitfalls

`scripts/run_opencode_afk.sh` runs opencode in JSON mode, stores the raw stream in temporary JSONL files, and renders a readable progress view to the terminal. The raw JSONL stream is used for session-id capture and verifier-result extraction, so formatting fixes must never mutate what is written to the output file. Only the terminal view should be sanitized.

## Do Not Trust The Live Stream Shape

The tempting approach is to assume `opencode run --format json` means each line received by `show_opencode_progress` is either a clean JSON object or plain wrapper text. That assumption is attractive because it works in small tests like piping a few hand-written JSON records into the function, and because the persisted opencode database stores clean `part` records.

The live terminal stream is messier than the persisted records. It may contain carriage returns, ANSI cursor movement, control-only lines, raw JSON events with leading display bytes, or malformed JSON-looking fragments. A line can fail `jq` even though the useful JSON object begins later in the same line. If the formatter simply prints every non-parseable line, raw events such as `{"type":"step_start",...}` leak to the terminal. If it prints control-only lines, the terminal cursor can be moved to the right and the next otherwise-correct summary appears indented.

The correct approach is to split raw capture from display rendering. Always append the original line to the JSONL file first. For display, remove carriage returns, try parsing the whole line with `jq`, then retry from the first `{` or `[` if parsing fails. If the recovered candidate still does not parse and the original line looks JSON-like, suppress it rather than printing it. Only non-JSON wrapper text such as `Starting opencode agent...` should be printed as plain text.

The important distinction is that the raw file is a data artifact, while terminal output is a hostile display surface. Preserve the former; aggressively normalize the latter.

## Do Not Stream Partial Text Or Trust Cursor Position

The tempting approach is to render model text incrementally by comparing the new text chunk with the previously rendered text and printing only the delta. That seems efficient and gives a live streaming feel. It also seems like the right fix when repeated model chunks would otherwise duplicate output.

That approach fails because terminal state is global. Tool summaries, model text, wrapper output, carriage returns, and ANSI cursor controls all share the same cursor. If the renderer prints a partial model delta without a newline, or if opencode/sbx writes a control sequence that leaves the cursor at column 80, the next `Finished tool: ...` line starts from that column. The visible symptom is misleading: it looks like the tool-summary jq filter generated spaces, but the persisted opencode `tool` parts can be perfectly clean.

The correct approach is to render only complete display lines and to reset the terminal line before every renderer-owned print. Use a single helper for every line emitted by `show_opencode_progress`:

```bash
print_progress_line() {
  printf '\r\033[K%s\n' "$1"
}
```

Model `text` and `reasoning` parts should be printed as complete lines. Strip ANSI escapes and carriage returns, drop everything from `<system-reminder>` onward, split multiline content into lines, trim leading whitespace, and skip whitespace-only lines. Do not compute terminal deltas. If duplicate cumulative chunks become a problem later, dedupe at the line/event level rather than by printing partial strings.

When diagnosing indentation, inspect actual opencode parts before changing formatter logic. The sandbox database can show whether stored parts are clean:

```bash
sbx exec -d opencode-tgreddit bash -lc 'python3 - <<"PY"
import sqlite3, json
sid = "<session-id>"
con = sqlite3.connect("file:/home/agent/.local/share/opencode/opencode.db?mode=ro", uri=True)
for pid, data in con.execute("select id, data from part where session_id=? order by time_created", (sid,)):
    obj = json.loads(data)
    if obj.get("type") in {"text", "reasoning", "tool"}:
        print(pid, repr(obj)[:1000])
PY'
```

If the database is clean but terminal output is indented, suspect live-stream control characters or control-only lines, not persisted tool metadata.

## Keep Tool Summaries Small And Error-Aware

The tempting approach is to build the display title from any useful-looking tool metadata. Real opencode tool parts often contain rich fields: previews, command output, file display text, truncation metadata, and descriptions. In a small sample, `state.metadata.preview` or `state.metadata.output` can look like a convenient summary.

That approach is unsafe. Real logs show metadata can contain full command output, large file previews, complete file contents, or injected reminder text. If a formatter uses broad `state.metadata.*` fields, progress lines become huge, multiline, or contaminated by content that was never meant to be a one-line title.

The correct title source order is deliberately narrow: `state.title`, `state.input.description`, `state.input.filePath`, first line of `state.input.command`, and, for failed tools only, first line of `state.error`. Do not use broad metadata fields for terminal titles. Sanitize the chosen title by stripping ANSI escapes, removing carriage returns, dropping everything from `<system-reminder>` onward, taking the first line, trimming leading whitespace, and truncating to a small maximum such as 140 characters.

Tool status should be explicit, especially for failures. The stored shape is `part.type == "tool"`, `part.tool`, and `part.state.status`. Map statuses like this:

```text
pending/running -> Running tool: <tool> - <title>
completed       -> Finished tool: <tool> - <title>
error           -> Tool failed: <tool> - <title>
unknown         -> Tool: <tool> - <title>
```

Errors often have no `state.title`, so the fallback path matters. A failed `read` might only have `state.input.filePath` plus `state.error`; a failed `bash` may only have `state.input.command`. The renderer should still produce a useful line such as `Tool failed: bash - cargo test` rather than either hiding the failure or dumping the entire state object.

## Verification Must Exercise The Renderer, Not Just The Shell

`bash -n scripts/run_opencode_afk.sh` and `shellcheck scripts/run_opencode_afk.sh` are necessary but insufficient. They validate shell syntax and common shell mistakes; they do not validate embedded jq filters against real opencode shapes, terminal control behavior, or the visible layout.

After touching `show_opencode_progress`, run a focused smoke test that includes the failure modes above: ANSI/control-only input before a tool event, JSON prefixed by terminal control bytes, multiline model text, `<system-reminder>` after normal text, completed tools with clean titles, and failed tools with no `state.title`. Inspect escaped output when testing control characters so it is obvious whether every rendered line starts by resetting the current terminal line.

A useful smoke-test shape is:

```bash
source <(awk '/^show_opencode_progress\(\)/,/^json_lines\(\)/ { if ($0 ~ /^json_lines\(\)/) exit; print }' scripts/run_opencode_afk.sh)
tmp=$(mktemp)
{
  printf '\033[80C\n'
  printf '\033[40C%s\n' '{"part":{"type":"tool","tool":"read","state":{"status":"completed","title":"file.md"}}}'
  printf '%s\n' '{"part":{"type":"text","text":"  Model chatter.\n<system-reminder>hide</system-reminder>"}}'
  printf '%s\n' '{"part":{"type":"tool","tool":"bash","state":{"status":"error","input":{"command":"cargo test\nsecond"},"error":"exit 101"}}}'
  printf '%s\n' 'Starting opencode agent in sandbox'
} | show_opencode_progress "$tmp" | python3 -c 'import sys; [print(repr(line.decode())) for line in sys.stdin.buffer]'
```

Expected escaped output should show each rendered line beginning with `\r\x1b[K`, and the visible text should be clean:

```text
Finished tool: read - file.md
Model chatter.
Tool failed: bash - cargo test
Starting opencode agent in sandbox
```

Finally, do a short live AFK run when possible. Some bugs only appear in the real combined stream from `sbx`, opencode, and the terminal; persisted opencode records and synthetic JSON tests can both look clean while the live terminal still smears output.

## Exit-Code / JSON Status Contract

`scripts/run_opencode_afk.sh` dispatches on the agent's final status. The agent writes that status as the last line of its final message:

- Implementer and code-quality agents emit `{"status":"pass"}`, `{"status":"needs-info"}`, or `{"status":"blocked"}`.
- The verifier emits the same statuses inside its JSON object (`pass`, `fail`, `needs-info`, `blocked`).

The harness owns the issue label. It maps `needs-info` to the `needs-info` label and `blocked` to the `blocked` label, appends the agent's final message or verifier `feedback` as a comment, clears transient state, and advances to the next runnable issue when running with `--all`. The agent never sets the label itself. If an implementer or code-quality run exits cleanly but no JSON status line is found, the harness treats it as `pass`.

The harness is workflow-only: it reads the status and the reason, not the agent's reasoning chain.
