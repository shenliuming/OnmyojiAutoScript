# Foster Platform Manual Recovery / Reserved Resource Safety

**Goal:** Resolve uncertain interrupted foster commands without double-assigning live platform resource slots or blindly replaying an action that may already have occurred in the game.

**Branch:** `feat/foster-platform`. Keep Python OAS unchanged. Make development commits without running GitHub Actions, then update `.github/ci-trigger/foster-platform` once for final verification.

## Current risk

An Agent's `INTERRUPTED` or missing command currently moves a foster job to `RECOVERY_REQUIRED` and immediately releases any platform reservation. The action may already have reached the game even when its terminal event was lost. Releasing its reserved slot early risks assigning that same physical slot to another customer.

## Invariants

- Uncertain delivery and interrupted execution retain a live `RESERVED` allocation until operator resolution or the resource cycle's natural end.
- `RECOVERY_REQUIRED` is never automatically dispatched or retried.
- Operator endpoints require the existing admin bearer token.
- Every resolution requires job id, current attempt, operator identifier, note, and explicit action.
- Reject a stale `expectedAttempt` or a job not presently `RECOVERY_REQUIRED`.
- Both resolution modes run in one MySQL transaction and write an audit record.
- `CONFIRM_SUCCEEDED` represents an operator-observed completed foster. On PLATFORM it requires a positive observed remaining duration and an active reservation. It marks Job SUCCESS, subscription success/next-run, and the allocation CONFIRMED with a bounded occupied-until time.
- `CONFIRM_NOT_EXECUTED` requires explicit operator confirmation that the old execution cannot still run. Release a RESERVED allocation exactly once, clear job's current allocation pointers, increment retry_count to prevent old Agent events from modifying the retry, and schedule RETRY at now + 5 minutes.
- Never release an already CONFIRMED slot as “not executed.” Resolve that inconsistency separately.
- A naturally expired resource cycle may be reaped; do not guess success after its allocation has already expired.
- Keep BASIC with no resource allocation supported.
- No customer-facing controls for manual recovery; admin only.

## Implementation

1. Add `0008_manual_recovery_audit.sql` with actor, note, attempt, resolution, observed remaining and optional prior allocation reference.
2. Remove early `release_for_job` from Agent reconnect reconciliation for interrupted/missing commands. Add regression coverage for retaining a real PLATFORM reservation.
3. Add `recovery` service and authenticated admin endpoints:
   - `GET /admin/recovery-jobs?limit=50`: RECOVERY_REQUIRED jobs, allocation/host metadata, no passwords.
   - `POST /admin/recovery-jobs/{job_id}/resolve`: JSON `{expectedAttempt, action, operator, note, observedRemainingSeconds?, confirmedStopped?}`.
4. Add MySQL integration tests: interrupted/missing preserve reservation, list requires auth, success finalizes and schedules, no-execution releases and advances attempt, wrong attempt and duplicate resolution reject, confirmed resource cannot be released as not executed, BASIC works.
5. Extend deployment guide with mandatory human verification steps.
6. At phase end run one complete CI (Rust fmt/check/tests/clippy, Python compileall, existing deployment syntax checks).

## Recovery procedure

The operator must check the game/emulator and Agent journal before resolving. If the foster is visible, record success and measured remaining seconds. If it did not happen and the previous automation process is confirmed stopped, mark not executed. If uncertain, leave the task in `RECOVERY_REQUIRED` with its reservation held; do not guess.
