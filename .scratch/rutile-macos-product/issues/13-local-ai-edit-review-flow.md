# Local-only AI edit review flow

Type: feature
Status: ready-for-agent
Labels: ready-for-agent
Parent: ../spec.md
Blocked by: 08

## What it delivers

An optional local-provider edit flow that returns bounded, reviewable diffs and
never silently mutates a document.

## Acceptance criteria

- The feature is opt-in, local-only, and unavailable without an authorized
  local provider.
- Requests carry bounded input, document identity, revision, and resource/time
  budgets.
- Results are proposed diffs with explicit accept, reject, and partial-accept
  actions.
- Acceptance revalidates the current revision and commits through the normal
  edit transaction contract.
- Timeouts, malformed output, provider failure, cancellation, and stale
  results leave the document unchanged and explain the outcome.
- No cloud endpoint, credential, model capability, or inference receipt is
  implied by tests.
