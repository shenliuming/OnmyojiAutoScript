# Foster Platform QR Login & Account Enrollment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the customer enrollment path from a newly created GameAccount to an ACTIVE emulator binding through a LoginSession, temporary QR delivery, detected account identity, SSE status updates, and explicit user confirmation.

**Architecture:** Rust Server remains authoritative for LoginSession state, account identity, and binding activation. Agent receives a StartLogin command for a reserved emulator slot and reports login progress, temporary QR data, and detected identity. Browser reads the public login status over HTTPS and receives state changes over SSE. This phase does not yet perform foster jobs. OAS/game-screen specifics remain behind the Agent-side login executor interface so the control plane can be tested with a fake executor first.

**Tech Stack:** Rust stable, Axum 0.8, Tokio, SQLx + MySQL 8, Serde, UUID, Chrono, SSE, existing WebSocket protocol and Agent runtime.

**Spec:** `docs/superpowers/specs/2026-09-21-onmyoji-foster-platform-design.md`

## Global Constraints

- Work only on `feat/foster-platform`; keep `master` clean for upstream sync.
- One GameAccount may have only one reserved/active binding path at a time.
- Starting a LoginSession consumes emulator capacity through an existing PENDING binding.
- LoginSession owns a short-lived public token; DB stores only SHA-256 token hash.
- QR data is temporary and must expire; no account password is stored.
- Browser cannot activate a binding merely by knowing the read-only login token.
- User confirmation is required after identity detection before the binding becomes ACTIVE.
- Activation must atomically:
  1. verify LoginSession is in `VERIFYING_ACCOUNT`;
  2. verify PENDING binding still belongs to that GameAccount;
  3. persist trusted identity factors;
  4. set binding ACTIVE;
  5. set `game_account.active_emulator_id`;
  6. set account verify/login status;
  7. mark LoginSession SUCCESS.
- A failed or expired LoginSession releases the PENDING binding slot.
- Identity conflict or ambiguous identity must never auto-activate.
- SSE is Server → Browser only.
- This phase uses a fake Agent login executor; real OAS QR navigation/screenshot recognition comes later in the OAS Bridge phase.
- Every behavior change follows RED → verify expected failure → minimal implementation → GREEN → full suite.

## Review Focus

- Two concurrent confirmation requests must activate at most once.
- Expired public token must not expose QR or account details.
- Read-only public token must not be accepted by the confirmation endpoint.
- Session expiry must release capacity.
- QR refresh must replace previous QR metadata rather than accumulating active QR values.
- Identity mismatch must stop before ACTIVE binding.
- A stale Agent event for an older LoginSession must not mutate the current session.

---

## File Structure

```text
foster-platform/
├── migrations/
│   └── 0002_login_session.sql
└── crates/
    ├── domain/src/
    │   └── login.rs
    ├── protocol/src/
    │   ├── server.rs
    │   └── agent.rs
    ├── server/src/
    │   ├── app.rs
    │   └── enrollment/
    │       ├── mod.rs
    │       ├── model.rs
    │       ├── repository.rs
    │       ├── service.rs
    │       ├── public_api.rs
    │       └── sse.rs
    └── server/tests/
        ├── login_session.rs
        ├── login_public_api.rs
        └── login_activation.rs
```

---

### Task 1: Add LoginSession domain states and schema

**Files:**
- Create: `foster-platform/crates/domain/src/login.rs`
- Modify: `foster-platform/crates/domain/src/lib.rs`
- Create: `foster-platform/migrations/0002_login_session.sql`
- Create: `foster-platform/crates/server/tests/login_session.rs`

**Domain states:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LoginSessionStatus {
    Created,
    WaitingEmulator,
    Preparing,
    WaitingQr,
    QrReady,
    WaitingScan,
    DetectingLogin,
    VerifyingAccount,
    Success,
    QrExpired,
    Failed,
    Cancelled,
}
```

**Schema:**

```sql
CREATE TABLE login_session (
    id BIGINT NOT NULL AUTO_INCREMENT,
    session_no VARCHAR(64) NOT NULL,
    game_account_id BIGINT NOT NULL,
    binding_id BIGINT NOT NULL,
    emulator_id BIGINT NOT NULL,

    status VARCHAR(32) NOT NULL,
    public_token_hash CHAR(64) NOT NULL,
    control_token_hash CHAR(64) NOT NULL,

    qr_payload TEXT NULL,
    qr_expires_at DATETIME(3) NULL,

    detected_masked_account VARCHAR(255) NULL,
    detected_character_name VARCHAR(64) NULL,
    detected_server_name VARCHAR(64) NULL,
    detected_game_uid VARCHAR(64) NULL,

    expires_at DATETIME(3) NOT NULL,
    confirmed_at DATETIME(3) NULL,
    failed_reason VARCHAR(255) NULL,

    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),

    PRIMARY KEY (id),
    UNIQUE KEY uk_login_session_no (session_no),
    UNIQUE KEY uk_login_public_token_hash (public_token_hash),
    UNIQUE KEY uk_login_control_token_hash (control_token_hash),
    KEY idx_login_account_status (game_account_id, status),
    KEY idx_login_expiry (status, expires_at),

    CONSTRAINT fk_login_game_account
        FOREIGN KEY (game_account_id) REFERENCES game_account(id),
    CONSTRAINT fk_login_binding
        FOREIGN KEY (binding_id) REFERENCES emulator_account_binding(id),
    CONSTRAINT fk_login_emulator
        FOREIGN KEY (emulator_id) REFERENCES emulator_instance(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
```

**Tests:**
- migration creates table;
- session_no unique;
- public/control token hashes unique;
- invalid FK binding rejected.

Commit: `feat: add login session state and schema`

---

### Task 2: Create LoginSession service and secure token handling

**Files:**
- Create: `foster-platform/crates/server/src/enrollment/mod.rs`
- Create: `model.rs`
- Create: `repository.rs`
- Create: `service.rs`
- Modify: `server/src/lib.rs`
- Modify: workspace/server dependencies for `sha2`, `base64`, `rand`
- Extend: `server/tests/login_session.rs`

**Interfaces:**

```rust
pub struct CreatedLoginSession {
    pub session_id: i64,
    pub session_no: String,
    pub public_token: String,
    pub control_token: String,
    pub emulator_id: i64,
    pub binding_id: i64,
}

pub async fn create_login_session(
    &self,
    game_account_id: i64,
    ttl: Duration,
) -> Result<CreatedLoginSession, EnrollmentError>;
```

**Behavior:**
1. call `BindingAllocator::allocate_pending`;
2. generate 32 random bytes for public token;
3. generate independent 32 random bytes for control token;
4. encode URL-safe base64 without padding;
5. persist SHA-256 hex hashes only;
6. insert LoginSession in `CREATED`;
7. on insert failure, mark PENDING binding UNBOUND before returning error.

**Tests:**
- raw token is not present in DB;
- public token and control token differ;
- PENDING binding exists and consumes capacity;
- second LoginSession creation for same account returns AlreadyBound;
- DB failure after binding allocation releases reservation.

Commit: `feat: create secure login sessions`

---

### Task 3: Extend Server ↔ Agent protocol for login flow

**Files:**
- Modify: `protocol/src/server.rs`
- Modify: `protocol/src/agent.rs`
- Add protocol serialization tests.

**Server commands:**

```rust
StartLogin(StartLoginCommand {
    session_no: String,
    game_account_id: i64,
    emulator_code: String,
})

CancelLogin(CancelLoginCommand {
    session_no: String,
})
```

**Agent events:**

```rust
LoginPreparing { session_no }
LoginQrReady {
    session_no,
    qr_payload,
    expires_at,
}
LoginQrExpired { session_no }
LoginIdentityDetected {
    session_no,
    masked_account,
    character_name,
    server_name,
    game_uid,
}
LoginFailed {
    session_no,
    code,
    message,
}
```

All events must serialize with stable uppercase tags.

**Tests:**
- `START_LOGIN` stable JSON tag;
- `LOGIN_QR_READY` roundtrip;
- `LOGIN_IDENTITY_DETECTED` optional fields roundtrip.

Commit: `feat: extend agent protocol for login enrollment`

---

### Task 4: Process login Agent events with strict session correlation

**Files:**
- Modify: `server/src/agent_gateway/handler.rs`
- Extend: `enrollment/repository.rs`
- Extend: `enrollment/service.rs`
- Create: `server/tests/login_agent_events.rs`

**Rules:**
- every login event carries `session_no`;
- Server loads session by `session_no`;
- event is accepted only when event comes from the Host owning `session.emulator_id`;
- terminal sessions ignore later Agent events;
- QR event only moves:
  - PREPARING/WAITING_QR/QR_EXPIRED → QR_READY;
- identity event only moves:
  - QR_READY/WAITING_SCAN/DETECTING_LOGIN → VERIFYING_ACCOUNT;
- stale event for another session does nothing.

**QR storage:**
- MVP stores `qr_payload` in DB only until `qr_expires_at`;
- payload may be a data URI or short-lived object URL;
- later OAS Bridge may replace this storage transport without changing public API.

**Tests:**
- valid QR event persists QR and expiry;
- newer QR replaces prior QR;
- wrong Host cannot mutate session;
- terminal SUCCESS session ignores late QR;
- identity event stores detected identity and transitions to VERIFYING_ACCOUNT.

Commit: `feat: process agent login events`

---

### Task 5: Add public login status API and SSE

**Files:**
- Create: `enrollment/public_api.rs`
- Create: `enrollment/sse.rs`
- Modify: `server/src/app.rs`
- Create: `server/tests/login_public_api.rs`

**Routes:**

```text
GET /public/login/:public_token
GET /public/login/:public_token/events
```

**Public response fields:**

```json
{
  "sessionNo": "...",
  "status": "QR_READY",
  "qrPayload": "...",
  "qrExpiresAt": "...",
  "characterName": null,
  "serverName": null,
  "expiresAt": "..."
}
```

Rules:
- lookup by SHA-256(public token);
- expired LoginSession returns 410 Gone;
- invalid token returns 404;
- QR payload only returned when status is QR_READY/WAITING_SCAN and QR not expired;
- never expose masked login account or UID on public read endpoint before confirmation;
- SSE emits named event `login_status`;
- send initial state immediately;
- then send on state version changes;
- heartbeat/comment every ~15 seconds.

MVP implementation may use an in-process broadcast/watch registry keyed by session id. DB remains source of truth.

**Tests:**
- valid token returns status;
- invalid token 404;
- expired session 410;
- expired QR is omitted;
- public endpoint does not expose masked account/UID;
- SSE immediately emits current state.

Commit: `feat: add login status api and sse`

---

### Task 6: Confirm detected account and atomically activate binding

**Files:**
- Extend: `enrollment/service.rs`
- Extend: `enrollment/repository.rs`
- Modify: `enrollment/public_api.rs`
- Create: `server/tests/login_activation.rs`

**Route:**

```text
POST /public/login/:control_token/confirm
```

The control token is independent from the public read token.

**Request:**

```json
{
  "confirmed": true
}
```

**Activation transaction:**
1. `SELECT login_session ... FOR UPDATE` by control token hash;
2. require status `VERIFYING_ACCOUNT`;
3. require `expires_at > NOW(3)`;
4. lock the PENDING binding;
5. require binding GameAccount/emulator match session;
6. lock GameAccount;
7. require account has no different `active_emulator_id`;
8. build `DetectedIdentity` from session fields;
9. if account already has trusted identity rows:
   - run `verify_identity`;
   - require `Verified`;
   - Mismatch/Ambiguous → do not activate;
10. if first enrollment:
   - require at least `character_name + server_name` or `game_uid`;
   - persist available factors with `source='LOGIN_ENROLLMENT'`;
11. set binding `ACTIVE`, `bound_at=NOW(3)`;
12. set `game_account.active_emulator_id`;
13. set `login_status='LOGGED_IN'`, `verify_status='VERIFIED'`;
14. set LoginSession `SUCCESS`, `confirmed_at=NOW(3)`;
15. commit.

**Tests:**
- first enrollment activates successfully;
- existing trusted identity match activates;
- UID conflict rejects activation;
- masked-only identity cannot activate;
- read-only public token cannot confirm;
- concurrent confirm calls yield one activation and one idempotent already-success response;
- session binding mismatch rejects activation.

Commit: `feat: activate verified game account binding`

---

### Task 7: Expire/cancel sessions and release reserved capacity

**Files:**
- Extend: `enrollment/service.rs`
- Extend: `enrollment/repository.rs`
- Add: `server/tests/login_expiry.rs`

**Behavior:**
- background sweeper finds non-terminal sessions where `expires_at <= NOW(3)`;
- transaction:
  - lock LoginSession;
  - if still non-terminal, set CANCELLED or FAILED with `SESSION_EXPIRED`;
  - if binding is PENDING, set it UNBOUND and set `unbound_at=NOW(3)`;
- QR expiry alone does NOT cancel LoginSession; it sets QR_EXPIRED and waits for Agent refresh;
- explicit cancel using control token releases PENDING binding.

**Tests:**
- session expiry releases capacity;
- QR expiry does not release account reservation;
- explicit cancel releases reservation;
- ACTIVE binding is never released by a stale session sweeper.

Commit: `feat: expire login sessions safely`

---

### Task 8: Dispatch StartLogin to the connected Agent

**Files:**
- Extend: `agent_gateway/registry.rs`
- Extend: `agent_gateway/handler.rs`
- Extend: `enrollment/service.rs`
- Create: `server/tests/login_dispatch.rs`

Registry gains a per-Host outbound sender owned by the current connection.

**Behavior:**
1. after LoginSession creation, resolve emulator → Host + emulator_code;
2. require Host currently online;
3. enqueue `ServerCommand::StartLogin`;
4. when Agent socket writer sends it successfully, move session to PREPARING;
5. if Host offline, keep session WAITING_EMULATOR and do not fail it;
6. on Host reconnect, enrollment dispatcher can retry waiting sessions.

**Tests:**
- online Host receives START_LOGIN;
- offline Host leaves WAITING_EMULATOR;
- replacement connection receives new command, old connection does not;
- duplicate dispatch for same session is idempotent.

Commit: `feat: dispatch login sessions to agents`

---

## Phase 3 Completion Gate

Do not begin Foster Scheduler until all are true:

- LoginSession schema and status enum exist.
- Session creation reserves one emulator slot.
- Raw public/control tokens are never persisted.
- Public and control tokens are independent.
- Login protocol messages serialize stably.
- Agent login events are correlated by session and owning Host.
- QR refresh replaces old QR metadata.
- Public API omits masked account/UID before confirmation.
- SSE emits initial and changed state.
- Read token cannot perform control actions.
- First enrollment requires strong enough identity.
- Existing account identity is verified before activation.
- UID/role/server conflicts do not activate.
- Binding activation and GameAccount update happen atomically.
- Concurrent confirm cannot double-activate.
- Expired/cancelled sessions release only PENDING reservations.
- START_LOGIN dispatch targets the correct current Agent connection.
- Full CI passes `fmt/check/test/clippy`.
