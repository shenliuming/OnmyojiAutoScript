# Foster Platform Scheduler & FosterJob Implementation Plan

> **Execution:** Continue the established TDD workflow on `feat/foster-platform`: RED test → verify expected failure → minimal implementation → GREEN → full CI.

**Goal:** Build the commercial scheduling core: subscriptions, quiet periods, manual pauses, due-job creation, per-account/per-emulator serialization, success cadence, deferral, and structured retry decisions.

**Architecture:** The Server owns scheduling. A subscription becoming due creates one durable `foster_job`; a dispatcher later decides whether the job may run. OAS remains uninvolved in scheduling. This phase stops before actual game automation commands.

**Spec:** `docs/superpowers/specs/2026-09-21-onmyoji-foster-platform-design.md`

## Global Rules

- Default cadence: 360 minutes after the **actual successful execution time**.
- If OAS later supplies a trustworthy remaining duration, success completion may use that instead of the configured interval.
- Quiet periods and manual pause outrank daily target count.
- No rapid “catch-up” runs after a deferral.
- A job is checked for quiet/manual pause both when created and immediately before dispatch.
- One GameAccount may have at most one executing job.
- One Emulator may have at most one executing job.
- SUCCESS is the only state counted as a successful foster.
- Identity mismatch and login expiry are non-blind-retry failures.
- Server remains the only authority for job state.

## Task 1 — Add scheduler domain types and schema

Create:

- `crates/domain/src/subscription.rs`
- `crates/domain/src/job.rs`
- `migrations/0003_scheduler.sql`
- `server/tests/scheduler_schema.rs`

Tables:

### foster_plan

- id
- plan_code unique
- plan_name
- daily_target_runs
- interval_minutes
- resource_mode: USER_FRIEND / PLATFORM
- resource_type nullable
- status

### foster_subscription

- id
- subscription_no unique
- game_account_id
- plan_id
- resource_mode snapshot
- resource_type snapshot nullable
- daily_target_runs
- interval_minutes
- status: PENDING_LOGIN / ACTIVE / PAUSED / EXPIRED / SUSPENDED
- last_success_at
- next_run_at
- manual_pause_until
- start_at
- end_at

Indexes:
- `(status, next_run_at)`
- `game_account_id`

### foster_quiet_period

- id
- game_account_id
- weekday_mask
- start_time
- end_time
- timezone default Asia/Shanghai
- before_buffer_minutes
- after_buffer_minutes
- enabled

### foster_job

- id
- job_no unique
- subscription_id
- game_account_id
- emulator_id nullable
- status
- scheduled_at
- deferred_until nullable
- started_at nullable
- finished_at nullable
- retry_count
- retry_after nullable
- error_code nullable
- result_message nullable
- remaining_seconds nullable
- screenshot_url nullable
- created_at / updated_at

States:

```text
PENDING
DEFERRED_QUIET
DEFERRED_MANUAL
WAITING_EMULATOR
WAITING_RESOURCE
SWITCHING_ACCOUNT
VERIFYING_ACCOUNT
RUNNING
SUCCESS
RETRY
FAILED
IDENTITY_MISMATCH
CANCELLED
RECOVERY_REQUIRED
```

Database guards:

1. one non-terminal scheduler-owned job per subscription using a generated nullable unique guard for statuses:
   `PENDING, DEFERRED_QUIET, DEFERRED_MANUAL, WAITING_EMULATOR, WAITING_RESOURCE, SWITCHING_ACCOUNT, VERIFYING_ACCOUNT, RUNNING, RETRY, RECOVERY_REQUIRED`;
2. one executing job per account for:
   `SWITCHING_ACCOUNT, VERIFYING_ACCOUNT, RUNNING`;
3. one executing job per emulator for those same states.

Tests:

- two active jobs cannot be inserted for one subscription;
- two RUNNING jobs cannot share account;
- two RUNNING jobs cannot share emulator;
- terminal history does not block new jobs.

Commit: `feat: add scheduler job schema`

---

## Task 2 — Implement quiet-period calculation as pure domain logic

Add dependency: `chrono-tz`.

Create pure API:

```rust
pub struct QuietWindow { ... }

pub enum ScheduleGate {
    Open,
    DeferredUntil(DateTime<Utc>),
}

pub fn evaluate_quiet_periods(
    now: DateTime<Utc>,
    timezone: Tz,
    windows: &[QuietWindow],
) -> ScheduleGate;
```

Rules:

- weekday mask is evaluated in the configured local timezone;
- support normal periods, e.g. 20:00–23:00;
- support cross-midnight periods, e.g. 23:00–01:00;
- before/after buffers are part of the protected interval;
- when multiple windows overlap, defer to the latest end;
- DST behavior must use timezone-aware conversion, not fixed +08 logic.

Tests:

- outside window → Open;
- within normal window → deferred to end + buffer;
- cross-midnight;
- weekday mask;
- before buffer;
- after buffer;
- overlapping windows.

Commit: `feat: add quiet period scheduling rules`

---

## Task 3 — Create due jobs transactionally

Create:

- `server/src/scheduler/mod.rs`
- `server/src/scheduler/repository.rs`
- `server/src/scheduler/service.rs`
- `server/tests/scheduler_due_jobs.rs`

API:

```rust
pub struct SchedulerService { ... }

pub async fn create_due_jobs(
    &self,
    now: DateTime<Utc>,
    limit: u32,
) -> Result<Vec<i64>, SchedulerError>;
```

Eligibility:

- subscription status ACTIVE;
- `start_at <= now < end_at`;
- `next_run_at <= now`;
- account has an ACTIVE emulator binding;
- no existing non-terminal job for subscription.

Transaction per claimed subscription:

1. lock due subscription;
2. re-check eligibility;
3. create FosterJob with `scheduled_at = original next_run_at`;
4. set subscription `next_run_at = NULL` while job owns this schedule cycle;
5. commit.

Concurrency test:

- two SchedulerService instances race on one due subscription;
- exactly one job is created.

Tests:

- not-yet-due ignored;
- expired subscription ignored;
- inactive subscription ignored;
- due subscription creates one job;
- duplicate scheduler ticks remain idempotent;
- two schedulers cannot double-create.

Commit: `feat: create due foster jobs transactionally`

---

## Task 4 — Apply manual pause and quiet deferral

Create service:

```rust
pub async fn gate_pending_job(
    &self,
    job_id: i64,
    now: DateTime<Utc>,
) -> Result<JobGateResult, SchedulerError>;
```

Priority:

1. invalid/disabled account → block/fail according to reason;
2. manual pause;
3. quiet period;
4. executable.

Manual pause:
- if `manual_pause_until > now`, job → DEFERRED_MANUAL and `deferred_until = manual_pause_until`.

Quiet:
- evaluate all enabled windows;
- job → DEFERRED_QUIET and store calculated UTC `deferred_until`.

On a later gate call when `now >= deferred_until`:
- move back to PENDING before continuing;
- do not create a second job.

Tests:

- manual pause outranks quiet period;
- quiet deferral records exact end;
- crossing midnight works through DB-loaded window;
- expired pause resumes same job;
- repeated gate is idempotent;
- no catch-up/duplicate job created.

Commit: `feat: defer foster jobs for account safety windows`

---

## Task 5 — Claim emulator execution slot atomically

API:

```rust
pub async fn claim_for_execution(
    &self,
    job_id: i64,
    now: DateTime<Utc>,
) -> Result<ClaimResult, SchedulerError>;
```

Before claim, repeat pause/quiet gate.

Requirements:

- GameAccount still has exactly the expected ACTIVE binding;
- Host/Emulator status permits execution;
- lock job + account binding + emulator;
- set `emulator_id`;
- transition `PENDING -> SWITCHING_ACCOUNT`.

If emulator is busy:
- job → WAITING_EMULATOR;
- do not fail.

Database unique executing guards are final protection against races.

Tests:

- two jobs for different accounts on same emulator: only one claims;
- same account cannot execute two jobs;
- offline emulator → WAITING_EMULATOR;
- pause beginning while waiting prevents later claim;
- quiet period beginning while waiting prevents later claim.

Commit: `feat: serialize foster execution by account and emulator`

---

## Task 6 — Add controlled job stage transitions

API:

```rust
pub async fn transition_job(
    &self,
    job_id: i64,
    expected: FosterJobStatus,
    next: FosterJobStatus,
    now: DateTime<Utc>,
) -> Result<bool, SchedulerError>;
```

Allowed execution path:

```text
SWITCHING_ACCOUNT
  -> VERIFYING_ACCOUNT
  -> RUNNING
  -> SUCCESS / RETRY / FAILED / IDENTITY_MISMATCH
```

Reject impossible backward transitions.

Set:
- `started_at` on first executing transition;
- `finished_at` for terminal states.

Tests:
- happy path;
- stale expected-state update returns false;
- RUNNING cannot jump back to SWITCHING_ACCOUNT;
- IDENTITY_MISMATCH is terminal.

Commit: `feat: enforce foster job state transitions`

---

## Task 7 — Complete success and calculate next schedule

API:

```rust
pub async fn complete_success(
    &self,
    job_id: i64,
    success_at: DateTime<Utc>,
    remaining_seconds: Option<i64>,
) -> Result<DateTime<Utc>, SchedulerError>;
```

Transaction:

1. lock RUNNING job;
2. lock subscription;
3. mark job SUCCESS;
4. set finished_at;
5. store remaining_seconds;
6. subscription.last_success_at = success_at;
7. calculate next:
   - valid `remaining_seconds > 0` → `success_at + remaining_seconds`;
   - otherwise `success_at + interval_minutes`;
8. set subscription.next_run_at;
9. commit.

No “daily catch-up” logic is added.

Tests:

- fallback 360-minute cadence;
- OCR remaining duration overrides configured cadence;
- next time is based on actual success, not original scheduled_at;
- completing same job twice is idempotent/rejected safely;
- only SUCCESS contributes to daily-success query.

Commit: `feat: schedule next foster after successful execution`

---

## Task 8 — Structured failures and retry policy

Create:

```rust
pub enum FosterErrorCode {
    NoSlot,
    ProviderNotFound,
    AccountLoginExpired,
    IdentityMismatch,
    EmulatorOffline,
    NetworkError,
    GameBusy,
    Unknown,
}

pub enum RetryDecision {
    RetryAt(DateTime<Utc>),
    WaitForEmulator,
    FailTerminal,
    SuspendAccount,
}
```

MVP policy:

- NO_SLOT → immediate/short retry later resource phase;
- PROVIDER_NOT_FOUND → retry with alternate provider later resource phase;
- ACCOUNT_LOGIN_EXPIRED → terminal current job + subscription SUSPENDED;
- IDENTITY_MISMATCH → job IDENTITY_MISMATCH + subscription SUSPENDED;
- EMULATOR_OFFLINE → WAITING_EMULATOR;
- NETWORK_ERROR → RETRY in configurable 5 minutes;
- GAME_BUSY → RETRY in 5 minutes;
- UNKNOWN → bounded retry count, then FAILED.

A terminal failure must restore subscription scheduling deliberately; never leave `next_run_at = NULL` forever.

Tests for every policy branch.

Commit: `feat: add foster retry policy`

---

## Task 9 — Add scheduler run-once orchestration

API:

```rust
pub async fn run_once(&self, now: DateTime<Utc>) -> Result<SchedulerRunReport, SchedulerError>;
```

Run order:

1. create due jobs;
2. find jobs whose deferral/retry time has arrived;
3. gate pending jobs;
4. claim available emulator slots;
5. return claimed job ids for the future OAS dispatch layer.

Do not dispatch OAS in Phase 4.

Tests:

- one complete scheduler tick;
- repeated tick is idempotent;
- deferred job resumes later;
- emulator contention leaves the second job waiting.

Commit: `feat: orchestrate foster scheduler ticks`

---

## Phase 4 Completion Gate

Before OAS Foster Bridge work:

- schema invariants pass;
- quiet calculations pass including cross-midnight;
- concurrent scheduler cannot double-create jobs;
- quiet/manual pause is rechecked before execution;
- one account and one emulator cannot execute concurrently;
- job state transitions reject stale/backward moves;
- SUCCESS sets next_run_at from actual success;
- no catch-up burst exists;
- login-expired and identity-mismatch suspend further scheduling;
- terminal failures never strand a subscription with NULL next_run_at accidentally;
- full CI passes fmt/check/test/clippy.
