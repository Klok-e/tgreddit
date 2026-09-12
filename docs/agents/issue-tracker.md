# Issue tracker: Local Markdown

Issues and specs for this repo live as markdown files in `.scratch/`.

## Conventions

- One feature per directory: `.scratch/<feature-slug>/`
- The spec is `.scratch/<feature-slug>/spec.md`
- Implementation issues are one file per ticket at `.scratch/<feature-slug>/issues/<NN>-<slug>.md`, numbered from `01`, never a single combined tickets file
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

## Wayfinding operations

Used by `/wayfinder` for decision tickets. These tickets use the lifecycle and dependency format below; implementation issues use the triage and AFK conventions above.

- **Map**: `.scratch/<effort>/map.md`, using the map body defined in `/wayfinder`.
- **Child ticket**: one file per ticket at `.scratch/<effort>/issues/<NN>-<slug>.md`, numbered from `01`, with the question in the body. A `Type:` line records `research`, `prototype`, `grilling`, or `task`. A `Status:` line records `open`, `claimed`, or `resolved`; new tickets start as `open`.
- **Blocking**: a `Blocked by: NN, NN` line near the top refers to ticket numbers within the same effort. Use `Blocked by: None` for no dependencies. A ticket is unblocked when every listed ticket exists and has `Status: resolved`.
- **Frontier**: scan the map's child tickets for `Status: open` and no unresolved dependencies; first by number wins.
- **Claim**: set `Status: claimed` and save before any work.
- **Resolve**: append the answer under an `## Answer` heading, set `Status: resolved`, then append a context pointer (gist + link) to the map's `Decisions so far` section.
