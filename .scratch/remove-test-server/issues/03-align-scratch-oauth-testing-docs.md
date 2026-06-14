Status: complete

# Align Scratch OAuth Testing Docs

## Parent

.scratch/remove-test-server/PRD.md

## What to build

Update the historical Reddit OAuth scratch planning docs so their testing expectations match the active testing policy. The docs should make clear that local HTTP servers pretending to be Reddit are forbidden, while preserving completion history and adding correction notes where previous completed work used the now-rejected test harness approach.

## Acceptance criteria

- [ ] The Reddit OAuth scratch PRD no longer recommends mocked HTTP responses or local fake Reddit servers for Reddit API behavior tests.
- [ ] The Reddit OAuth scratch PRD states that normal Reddit tests should use pure fixtures or in-process helpers without network I/O.
- [ ] The Reddit OAuth scratch PRD states that network-dependent Reddit behavior belongs in ignored live Reddit integration tests.
- [ ] The completed OAuth transport issue includes a correction note explaining that the local HTTP test harness violated the current testing policy and is being removed.
- [ ] Existing historical completion notes are preserved rather than rewritten as if the violation never happened.
- [ ] No application behavior changes are made as part of this documentation-only issue.

## Blocked by

None - can start immediately

## Comments

### AFK completed

Aligned the historical Reddit OAuth scratch PRD with the active testing policy and added a correction note to the completed OAuth transport issue while preserving its original completion history.
