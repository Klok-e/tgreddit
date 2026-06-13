Status: ready-for-agent

# Remove Fake Reddit Server

## Parent

.scratch/remove-test-server/PRD.md

## What to build

Remove the local fake Reddit HTTP server from normal tests and keep OAuth transport coverage at pure deterministic seams. The normal test suite should no longer start a local socket pretending to be Reddit, and production code should not retain a custom base URL constructor if it only existed for that forbidden test harness.

## Acceptance criteria

- [ ] The local fake Reddit HTTP server module is deleted.
- [ ] The normal OAuth test that depends on the fake Reddit HTTP server is removed.
- [ ] The test-only custom Reddit base URL constructor is removed if no supported use remains.
- [ ] Pure OAuth tests for token response parsing and default OAuth URL construction still pass.
- [ ] Normal tests do not add or use any local HTTP server that pretends to be Reddit.
- [ ] `cargo fmt`, `cargo clippy`, and `cargo test` pass.

## Blocked by

None - can start immediately
