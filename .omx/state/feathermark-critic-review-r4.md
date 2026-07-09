# FeatherMark Critic Review — Round 4

**Verdict:** ITERATE
**Reviewed SHA-256:** `6a3e336a1a71cfc400cb69ae8e4b3bd74c8cd1c7a56fe496f3fbb33e2a3cfb83`

## Final bounded revision

1. Assign `feathermark-types` and `feathermark-protocol` to Task 1A everywhere; Task 1B owns only core document/editor implementation.
2. Make pre-package `release-size.json` executable-only. After package creation/hashing, add exact package-size evidence enforcing 20 MiB before installed smoke.
3. Add unique five-runner release artifacts, exact fan-in, and one global completeness/hash/scenario assertion; repeat for package-smoke with package-size evidence in the chain.
4. Pin/install/check `tokei`, build locked release `xtask` before first use, and replace runner placeholders with one closed five-row capture/verify command.
5. Define `no_scroll = source_max_top == 0 || preview_max_y == 0` as the first branch in both directions and apply it to oracle, grading, endpoints, and asymmetric short-document tests.

The architecture, principles, alternatives, ADR, security, native Wayland, performance methodology, IME path, dependency direction, and Ferrite comparator are otherwise coherent. No new scope is required.
