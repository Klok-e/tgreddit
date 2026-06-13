# Issue tracker: Local Markdown

Issues and PRDs for this repo live as markdown files in `.scratch/`.

## Conventions

- One feature per directory: `.scratch/<feature-slug>/`
- The PRD is `.scratch/<feature-slug>/PRD.md`
- Implementation issues are `.scratch/<feature-slug>/issues/<NN>-<slug>.md`, numbered from `01`
- Triage state is recorded as a `Status:` line near the top of each issue file (see `triage-labels.md` for the role strings)
- Dependencies are recorded under a `## Blocked by` heading. Use `None` for no dependencies, or one bullet per blocking issue path.
- Comments and conversation history append to the bottom of the file under a `## Comments` heading
- AFK runner autonomy is fixed inside `scripts/run_codex_afk.sh` for spawned Codex calls only: workspace-write, no approval prompts, and network enabled. The network proxy allowlist lives in `.codex/config.toml` and applies project-wide.
- AFK runner state is local and resumable. If a Codex run exits mid-loop, rerun `scripts/run_codex_afk.sh`; it resumes from `.scratch/<feature>/.afk-state.json`.
- Live Telegram E2E tests are ignored Rust integration tests. Agents may run the relevant ignored test when an issue needs live delivery validation and local `tgreddit.toml` plus `telegram-e2e.toml` are present.

## Runnable AFK issues

`scripts/run_codex_afk.sh` only picks an issue when:

- Its `Status:` line is exactly `ready-for-agent`.
- Every path listed under `## Blocked by` exists.
- Every blocking issue has `Status: complete`.

The runner skips unresolved issues instead of marking them blocked. Prompt-injection defense for network-capable AFK runs comes from the script's sandbox flags plus the project Codex config's network allowlist, not from agent judgment.

If no AFK state exists, the runner starts the first runnable issue even when the worktree is dirty. Agents must inspect the current diff before changing code.

## When a skill says "publish to the issue tracker"

Create a new file under `.scratch/<feature-slug>/` (creating the directory if needed).

## When a skill says "fetch the relevant ticket"

Read the file at the referenced path. The user will normally pass the path or the issue number directly.
