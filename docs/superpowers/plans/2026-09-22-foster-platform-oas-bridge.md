# Foster Platform OAS Foster Bridge Implementation Plan

> **Execution:** Continue on `feat/foster-platform`. Development commits do not trigger CI. Run one final full CI by updating `.github/ci-trigger/foster-platform`.

**Goal:** Connect a claimed Rust `FosterJob` to the existing Python OAS automation so one BASIC foster operation can be executed end-to-end and reported back to the Server with structured stages, success timing, identity safety, and failure codes.

**Architecture:** Server remains authoritative. Server sends one `ExecuteFoster` command over the existing Agent WebSocket. Agent calls a localhost OAS HTTP bridge. OAS performs account selection plus one KekkaiUtilize foster attempt and returns a structured result. Agent converts the OAS response to protocol events. Server applies those events through `SchedulerService`.

**Phase 5 scope:** BASIC / `USER_FRIEND` is executable. Protocol fields reserve provider/resource targeting for the later resource-pool phase, but PLATFORM execution is rejected as unsupported until resource allocation exists.

## Global Rules

- Do not start or reuse the OAS scheduler loop for commercial scheduling.
- One command executes exactly one foster attempt.
- Do not store or transmit account passwords.
- Account switching uses existing logged-in account list + masked account/OCR aliases + character/server hints.
- Identity ambiguity must fail closed before foster.
- Server is the only authority for FosterJob state.
- OAS does not calculate the next commercial schedule.
- OAS may return trustworthy remaining seconds; Server decides `next_run_at`.
- HTTP bridge listens on localhost only in deployment.
- Existing OAS scheduled `KekkaiUtilize` behavior remains compatible.
- No GitHub Actions during intermediate commits; final trigger only.

## Task 1 — Extend protocol for foster execution

Add:

- `ExecuteFosterCommand`
- `FosterTargetIdentity`
- `FosterStage`
- `FosterStageChanged`
- `FosterSucceeded`
- `FosterFailed`

Command fields:

- job_id
- game_account_id
- emulator_code
- resource_mode
- resource_type nullable
- provider_alias nullable
- target_identity

Events:

- SWITCHING_ACCOUNT
- VERIFYING_ACCOUNT
- RUNNING
- success: remaining_seconds + screenshot_url nullable
- failure: FosterErrorCode + message + screenshot_url nullable

Stable uppercase serde tags.

## Task 2 — Add one-shot OAS foster bridge

Create:

- `module/foster_bridge/models.py`
- `module/foster_bridge/service.py`
- `module/foster_bridge/router.py`
- `tasks/KekkaiUtilize/foster_once.py`

Modify:

- `module/server/app.py`
- `tasks/KekkaiUtilize/script_task.py`

HTTP:

`POST /foster/execute`

Request:

- config_name
- job_id
- resource_mode
- resource_type
- provider_alias
- masked_account
- account_aliases
- character_name
- server_name
- game_uid

Response:

- success
- code
- message
- remaining_seconds
- detected identity fields
- screenshot_url nullable

Behavior:

1. reject PLATFORM until provider targeting is implemented;
2. instantiate existing OAS Config + Device for the supplied local config;
3. use existing SwitchAccount with masked account/aliases + character/server;
4. fail closed if account/character cannot be selected;
5. execute one foster attempt only;
6. map no suitable card → PROVIDER_NOT_FOUND;
7. no slot → NO_SLOT;
8. enter-realm timeout/navigation ambiguity → GAME_BUSY;
9. on success OCR own foster remaining duration;
10. never call `set_next_run` from the one-shot bridge.

Existing scheduled KekkaiUtilize remains behavior-compatible.

## Task 3 — Add Agent FosterExecutor and localhost HTTP client

Create:

- `agent/src/foster/mod.rs`
- `agent/src/foster/executor.rs`
- `agent/src/foster/fake.rs`
- `agent/src/foster/http.rs`

Add `reqwest`.

`FosterExecutor` accepts `ExecuteFosterCommand` and returns structured execution.

HTTP executor:

- resolves emulator_code → OAS config name from local mapping;
- POSTs to localhost OAS;
- timeout is configurable;
- no password data exists in request;
- translates OAS error code to FosterErrorCode;
- transport error → NETWORK_ERROR.

## Task 4 — Wire Agent runtime foster commands/events

On `ExecuteFoster`:

1. emit stage SWITCHING_ACCOUNT;
2. executor starts account switch;
3. emit stage VERIFYING_ACCOUNT once target identity is accepted by OAS;
4. emit stage RUNNING before foster attempt;
5. emit FosterSucceeded or FosterFailed.

For MVP the executor returns stage checkpoints so fake tests can verify event ordering.

Agent HELLO capabilities adds `FOSTER_EXECUTION`.

## Task 5 — Add Server Foster dispatcher

Create:

- `server/src/foster_dispatch/mod.rs`
- `server/src/foster_dispatch/repository.rs`
- `server/src/foster_dispatch/service.rs`

For a claimed `SWITCHING_ACCOUNT` job:

1. lock/read job + emulator + host + subscription;
2. load enabled trusted identities;
3. build target identity;
4. require enough identity hints to fail closed;
5. send `ExecuteFoster` through AgentRegistry;
6. offline/delivery failure → use existing `EMULATOR_OFFLINE` policy.

No duplicate dispatch for a terminal job.

## Task 6 — Process Agent foster events on Server

Agent gateway routes foster events to FosterDispatchService.

Stage event mapping:

- SWITCHING_ACCOUNT is acknowledgement only;
- VERIFYING_ACCOUNT: `SWITCHING_ACCOUNT -> VERIFYING_ACCOUNT`;
- RUNNING: `VERIFYING_ACCOUNT -> RUNNING`.

Success:

- requires RUNNING;
- call `complete_success(job_id, completed_at, remaining_seconds)`;
- store screenshot_url.

Failure:

- normalize code;
- if job is SWITCHING_ACCOUNT or VERIFYING_ACCOUNT, first normalize into RUNNING only when safe OR provide a failure transition path that does not fake execution;
- apply existing retry/suspend policy;
- store screenshot_url.

Failure handling must work from all execution stages without forcing an invalid success path.

## Task 7 — Tests and final verification

Rust tests:

- protocol serialization;
- fake Agent executor event order;
- Server dispatch builds identity without passwords;
- offline Agent causes WAITING_EMULATOR;
- success updates next_run_at;
- identity mismatch suspends;
- transport/network failure retries;
- duplicate terminal event is ignored safely.

Python:

- pure one-shot outcome mapping tests where possible;
- `python -m compileall module/foster_bridge tasks/KekkaiUtilize/foster_once.py`.

Final CI:

- cargo fmt
- cargo check
- cargo test --workspace --all-targets
- cargo clippy -D warnings
- Python compileall

## Phase 5 Completion Gate

- A claimed BASIC job can be serialized to ExecuteFoster.
- Agent can call localhost OAS bridge.
- OAS bridge does not run the OAS commercial scheduler.
- Account selection uses masked account / aliases / character / server; no password.
- Identity mismatch fails closed.
- One foster attempt returns structured success/failure.
- Remaining seconds flows back to Server success scheduling.
- NO_SLOT / PROVIDER_NOT_FOUND / GAME_BUSY / NETWORK_ERROR flow into the Phase 4 retry policy.
- Agent offline leaves job recoverable.
- PLATFORM is explicitly unsupported until resource-pool allocation exists.
- Final full CI passes once at phase end.
