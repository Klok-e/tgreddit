Status: ready-for-agent

# AFK Blocker Skip

## Problem Statement

The AFK harness runs an agent on a picked issue, takes the agent's last
message as signal, and either commits or retries. There is no workflow
path for an agent to surface a blocker that the agent itself cannot
resolve: missing local credentials, an ambiguous spec, an unsafe change,
or an environment the operator must set up. When an agent hits one of
these, it can only describe the blocker in its final message, which the
harness does not act on. The harness then either commits partial work or
burns cycles on the same blocker.

This was visible in the last AFK run: the implementer completed its
work but could not exercise the live Telegram E2E suite because the
local Reddit OAuth credentials were not present. The agent's final
message described the gap. The harness did not act on it. The next agent
in the pipeline ran unit tests only and the work was committed.

## Solution

Give an agent a way to mark the current issue as blocked, with the
harness acting on the mark. The agent signals blocker state via process
exit code. The harness interprets the code, sets the issue's triage
label, advances to the next runnable issue, and stops retrying the
current one. The operator resumes the skipped issue by re-labeling it.

The pipeline gains a non-retried terminal state for each issue, distinct
from pass and fail.

## User Stories

1. As the AFK operator, I want an agent to be able to stop its own run
   with a clear "I cannot proceed" signal, so that the harness does not
   silently retry or commit partial work.
2. As the AFK operator, I want a "needs info" terminal state, so that an
   agent waiting on missing local credentials or human context does not
   consume further cycles.
3. As the AFK operator, I want a "blocked" terminal state, so that an
   issue that is fundamentally unresolvable by the agent does not
   consume further cycles and is clearly marked.
4. As the AFK operator, I want the harness to advance to the next
   runnable issue after a blocker, so that a single stuck issue does not
   halt the whole loop.
5. As the AFK operator, I want the harness to set the issue's triage
   label to match the blocker kind, so that the issue's status in the
   tracker reflects the agent's signal.
6. As the AFK operator, I want the agent's final message to be
   preserved as the blocker reason, so that I can read what the agent
   saw when I triage the issue.
7. As the AFK operator, I want to resume a skipped issue by re-labeling
   it, so that the human-in-the-loop is the only way blocked work
   restarts.
8. As the AFK operator, I want the existing pass and fail paths
   unchanged, so that retry-on-fail and commit-on-pass still work as
   before.
9. As an agent, I want a single, simple signal (exit code) to
   communicate blocker state, so that I do not need to know the
   harness's label or commit internals.
10. As an agent, I want the agent prompt to spell out the exit-code
    contract, so that I emit the correct code for each blocker kind.

## Implementation Decisions

- The agent-to-harness signal is the process exit code. No JSON
  envelope, no marker file, no shared log path.
- The exit-code contract has four states: pass, fail, needs-info,
  blocked. The mapping is recorded in the agent prompts and in the
  harness dispatch.
- The harness owns the issue label. The agent never sets, edits, or
  removes the issue's triage label. The harness sets the label based on
  the exit code.
- The harness advances to the next runnable issue on the blocker exit
  codes. It does not retry the current issue. It does not halt the
  loop.
- The harness preserves the agent's final message as the blocker reason
  in the AFK run log, so the operator can read it during triage.
- The existing pass and fail paths are unchanged in behavior. Pass
  still commits and marks the issue complete. Fail still retries in the
  next cycle.
- The agent prompts are updated with one new line: when the agent is
  blocked, ambiguous, unsafe, or missing required local configuration,
  it exits with the appropriate code and explains in its final message.
- The harness script gains one new dispatch branch: on the blocker exit
  codes, set the issue label and advance.
- The "AFK script pitfalls" doc gains one note: the exit-code contract,
  the harness's ownership of the label, and the rule that the harness
  is workflow-only and does not inspect agent reasoning.

## Testing Decisions

- The harness dispatch is the seam to test. The harness script already
  exercises the pass and fail paths in the live runner; the new branch
  can be exercised by feeding each exit code in a dry run.
- A good test asserts the harness's behavior on each exit code: which
  label is set, whether the next issue is picked, whether the current
  issue is retried. It does not assert the agent's reasoning or the
  contents of the agent's final message.
- Prior art: the harness already branches on the verifier's pass/fail
  result. The new branch follows the same shape.
- The agent prompt changes are verified by reading the diff against the
  original "blocked or missing local secrets" line. No new automated
  test for the prompt content itself; prompt contract is checked at
  code review.

## Out of Scope

- Per-issue metadata for forcing a particular test path.
- Automatic re-poll of the tracker for label changes. The operator
  re-labels manually.
- A separate test-result database. Agent logs are written by the agent
  itself; the harness does not curate them.
- Splitting the code-quality gate from the verifier gate in the cycle.
  That is a separate rework.
- Detecting which kind of blocker applies. The agent picks the kind;
  the harness does not classify.
- Surfacing blocker reasons to channels other than the AFK run log. The
  operator reads the run log.

## Further Notes

The root cause of the last AFK run committing partial work was that the
agent's blocker signal had no machine-readable form. This PRD adds that
signal at the cheapest possible layer (exit code) and the most local
possible layer (the agent prompt + harness dispatch), without
introducing a shared protocol or a new test framework.
