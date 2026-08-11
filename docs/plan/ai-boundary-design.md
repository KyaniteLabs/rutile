# Local AI Editing Boundary (Roadmap 13) — Design Decision

Status: **deferred (explicit decision: AI remains out of scope until the
non-AI roadmap is stable)**. Parent issue:
`.scratch/rutile-macos-roadmap/issues/13-local-ai-editing-boundary.md`.
Blocked by 03 (LOCKED), 08 (DONE).

## Decision: DEFERRED

**AI capabilities do not enter the personal macOS product until the non-AI
roadmap (issues 01-12) is fully stable and shipped.** This is the explicit
answer to the issue's own question: "whether AI remains deferred until the
non-AI roadmap is stable."

## Rationale

1. **Trust surface**: AI introduces a new trust boundary (model outputs
   entering the document). The current security-core fence is designed for
   user-authored content. Adding AI requires extending the fence to validate
   model-generated edits — a significant security architecture change.

2. **Transport**: Local-only AI (on-device model) is the only acceptable
   transport for a privacy-first personal editor. On-device models require
   ML runtime dependencies (CoreML, ONNX, or similar) that conflict with
   the "boring, explicit code" engineering principle. Remote AI requires
   network transport + data retention policies — a fundamental privacy
   regression.

3. **Scope**: The issue asks about structure-from-mess, rewrite/tone,
   ghost completion, reading aids, export steering, and chance styling.
   Export steering (C7) and chance styling (C8) are already implemented
   without AI — they use deterministic algorithms. The remaining AI
   capabilities are speculative features that add complexity without
   clear user demand for a personal editor.

4. **Failure modes**: AI model availability failure, edit cap enforcement,
   preview/accept/reject semantics — each requires careful design and
   implementation. These are not blocking issues, but they're substantial
   enough to warrant their own focused design cycle when the time comes.

## Boundary contract (for future implementation)

When AI is eventually added, the following contract MUST hold:

1. **Local-only by default**: No network calls without explicit opt-in.
2. **Edit caps**: AI edits are bounded by `MAX_AI_EDIT_BYTES` per operation.
3. **Preview/accept/reject**: Every AI edit is a diff the user previews;
   rejected edits leave the document untouched.
4. **Security-core fence**: AI outputs pass through the same `SafeLinkTarget`
   and `render.rs` validation as user content. No bypass.
5. **Consent**: Each AI operation requires explicit user invocation — no
   background processing.
6. **No data retention**: AI processing is stateless — no conversation
   history, no training data collection.

## What this means for G005

Issue 13 is resolved as DEFERRED. The `{11|12|13}` critical path tail is
completed by issues 11 (revision-history contract) and 12 (local-search +
backlinks contract). AI is not needed for the personal editor to be a
complete daily-driver tool.
