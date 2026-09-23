# Foster Platform Command Journal / Reconnect Reconciliation Plan

**Goal:** Prevent duplicate foster execution across WebSocket retries, reconnects, and Agent process restarts, and move uncertain interrupted jobs to RECOVERY_REQUIRED instead of blindly re-running them.

## Core guarantees

1. Same FosterJob + attempt uses one stable Server command_id.
2. Agent persists command execution state locally.
3. Duplicate RUNNING command never starts a second OAS execution.
4. Duplicate FINISHED command replays cached terminal result without executing OAS.
5. Foster execution outbox survives WebSocket reconnect inside the same Agent process.
6. Agent restart converts persisted RUNNING commands to INTERRUPTED.
7. Agent HELLO reports command states.
8. Server reconciliation:
   - matching RUNNING command + executing job => keep running;
   - FINISHED command => allow cached terminal event replay;
   - INTERRUPTED matching command => Job -> RECOVERY_REQUIRED;
   - Server VERIFYING_ACCOUNT/RUNNING with no matching Agent command => RECOVERY_REQUIRED.
9. Terminal/retry jobs remain protected by existing job attempt/state guards.

## Task 1 — Protocol command reconciliation state

Extend AgentHello:

- command_states: Vec<AgentCommandState>, serde default

Add:
- AgentCommandKind: FOSTER / LOGIN
- AgentCommandStatus: RUNNING / FINISHED / INTERRUPTED
- AgentCommandState:
  - command_id
  - kind
  - job_id nullable
  - attempt nullable
  - session_no nullable
  - updated_at

## Task 2 — Stable Server command IDs

AgentRegistry adds:

- send_command_with_id(host_id, command_id, command)
- existing send_command remains for ephemeral Ping/Refresh commands

Foster dispatcher derives deterministic UUID v5 from:

`foster:{job_id}:{attempt}`

The same job attempt always gets the same command_id.

## Task 3 — Persistent Agent CommandJournal

Add `agent/src/command_journal/`.

Journal entry:

- command_id
- execution_key
- kind
- status
- job_id / attempt / session_no
- updated_at
- terminal_event nullable

Production journal path:

`FOSTER_COMMAND_JOURNAL_PATH`

default:

`./command-journal.json`

Startup behavior:

- load valid file;
- RUNNING from previous process -> INTERRUPTED;
- persist normalization atomically via temp file + rename;
- keep bounded recent FINISHED entries.

Decision API:

- START_NEW
- ALREADY_RUNNING
- REPLAY_FINISHED(event)
- INTERRUPTED

Extra defense: dedupe by execution_key `foster:{job_id}:{attempt}` even if a buggy Server sends a different command_id.

## Task 4 — Connection-independent Agent outbox

Replace per-WebSocket background event channel with runtime-wide AgentEventOutbox.

Background OAS execution writes to shared outbox.

A new WebSocket connection drains pending events.

This prevents a foster result from being lost merely because the socket reconnected.

On reconnect, cached FINISHED terminal events may be replayed; Server handlers are idempotent.

## Task 5 — Runtime journal integration

For ExecuteFoster:

1. inspect command_id + execution_key;
2. NEW -> mark RUNNING, spawn executor once;
3. RUNNING duplicate -> do not spawn;
4. FINISHED duplicate -> replay cached terminal event;
5. INTERRUPTED -> do not execute; reconciliation handles it.

On final success/failure:
- persist FINISHED + terminal event before enqueueing terminal event.

HELLO includes journal command_states.

Capabilities adds COMMAND_JOURNAL_V1.

## Task 6 — Server reconnect reconciliation

Add server reconciliation service.

On valid HELLO, after Host is authenticated and before normal event processing:

- read command_states;
- list executing jobs for host;
- INTERRUPTED matching foster command -> RECOVERY_REQUIRED;
- VERIFYING_ACCOUNT/RUNNING job without matching RUNNING/FINISHED command -> RECOVERY_REQUIRED;
- matching RUNNING stays unchanged;
- matching FINISHED stays unchanged while terminal event replay is accepted;
- do not auto retry RECOVERY_REQUIRED.

Store:
- error_code = AGENT_RECOVERY_REQUIRED
- result_message with reconciliation reason

SWITCHING_ACCOUNT is not automatically failed solely because it is absent from HELLO; dispatcher may safely resend the same deterministic command_id.

## Task 7 — Tests

Agent:
- same RUNNING command executes executor once;
- FINISHED duplicate replays result without re-execution;
- different command_id with same foster execution_key is deduped;
- persisted RUNNING becomes INTERRUPTED after reload;
- event outbox survives socket reconnect.

Server:
- deterministic command_id stable for same attempt;
- different attempt produces different command_id;
- RUNNING matching HELLO keeps job;
- INTERRUPTED matching HELLO -> RECOVERY_REQUIRED;
- RUNNING/VERIFYING job absent from HELLO -> RECOVERY_REQUIRED;
- FINISHED matching state does not force recovery.

## Final gate

Intermediate commits do not trigger Actions.

At phase end update `.github/ci-trigger/foster-platform` once and require:

- cargo fmt
- cargo check
- cargo test --workspace --all-targets
- cargo clippy -D warnings
- existing Python / provider / compose / PowerShell checks
