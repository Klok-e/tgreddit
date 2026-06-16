# Issue tracker: Local Markdown

Issues and PRDs for this repo live as markdown files in `.scratch/`.

## Conventions

- One feature per directory: `.scratch/<feature-slug>/`
- The PRD is `.scratch/<feature-slug>/PRD.md`
- Implementation issues are `.scratch/<feature-slug>/issues/<NN>-<slug>.md`, numbered from `01`
- Triage state is recorded as a `Status:` line near the top of each issue file (see `triage-labels.md` for the role strings)
- Dependencies are recorded under a `## Blocked by` heading. Use `None` for no dependencies, or one bullet per blocking issue path.
- Comments and conversation history append to the bottom of the file under a `## Comments` heading
- AFK runner state, when used, is stored locally at `.scratch/<feature>/.afk-state.json`.

## Runnable AFK issues

The AFK harness only picks an issue when:

- Its `Status:` line is exactly `ready-for-agent`.
- Every path listed under `## Blocked by` exists.
- Every blocking issue has `Status: complete`.

The runner skips unresolved issues instead of marking them blocked.

If no AFK state exists, the runner starts the first runnable issue even when the worktree is dirty. Agents must inspect the current diff before changing code.

## When a skill says "publish to the issue tracker"

Create a new file under `.scratch/<feature-slug>/` (creating the directory if needed).

## When a skill says "fetch the relevant ticket"

Read the file at the referenced path. The user will normally pass the path or the issue number directly.
