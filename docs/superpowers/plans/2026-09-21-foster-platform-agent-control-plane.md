# Foster Platform Agent WebSocket Control Plane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a versioned, authenticated Server ↔ Agent WebSocket control plane that persists Host/Emulator presence, sends heartbeats and emulator snapshots, and reconnects safely after network loss.

**Architecture:** Rust Server remains the authority for Host and Emulator state. Each Windows Agent initiates one outbound WebSocket connection, authenticates with an Agent token, sends HELLO plus emulator snapshots, and then maintains heartbeat/reconnect behavior. This phase uses a FakeEmulatorDriver only; real MuMu/LDPlayer integration and all game/OAS execution remain out of scope.

**Tech Stack:** Rust stable, Axum 0.8, Tokio, tokio-tungstenite, SQLx + MySQL 8, Serde/serde_json, UUID, Chrono, DashMap, async-trait, Tracing.

**Spec:** `docs/superpowers/specs/2026-09-21-onmyoji-foster-platform-design.md`

## Global Constraints

- Work only on `feat/foster-platform`; do not modify `master`.
- Existing Python OAS behavior remains untouched.
- Agent initiates WebSocket connections; Windows Hosts require no inbound public port.
- Server is the authority for Host/Emulator business state.
- Protocol messages use `protocol_version = 1` and explicit stable serialized type names.
- Production heartbeat interval is 15 seconds.
- Production offline timeout is 45 seconds.
- Invalid Agent tokens must never register a logical connection.
- Unknown `host_id` must never be auto-created from a HELLO.
- A replacement connection for the same Host replaces the old logical presence; the old socket closing must not remove the new presence.
- Emulator snapshot upserts must not overwrite Server-controlled `max_account_count` or `current_job_id`.
- Phase 2 does not implement LoginSession, QR, FosterJob, scheduling, resource allocation, H5, OAS bridge, or a real emulator vendor driver.
- Every behavior change follows RED → verify failure → minimal implementation → GREEN → full suite.

## Review Focus

- Old socket closes after a replacement socket is registered: the new presence must remain online.
- An Agent presents a valid global token but an unknown host id: Server must reject HELLO and create no Host row.
- Protocol version differs from 1: Server must reject the message before registering presence.
- Heartbeats stop after a valid HELLO: Host must become OFFLINE after the configured timeout, without deleting its database record.
- Emulator snapshot for an existing emulator must update Agent-observed fields but preserve `max_account_count` and `current_job_id`.

---

## File Structure

```text
foster-platform/
└── crates/
    ├── protocol/
    │   └── src/
    │       ├── lib.rs
    │       ├── envelope.rs
    │       ├── server.rs
    │       └── agent.rs
    ├── server/
    │   ├── src/
    │   │   ├── main.rs
    │   │   ├── app.rs
    │   │   ├── config.rs
    │   │   ├── lib.rs
    │   │   └── agent_gateway/
    │   │       ├── mod.rs
    │   │       ├── auth.rs
    │   │       ├── handler.rs
    │   │       └── registry.rs
    │   └── tests/
    │       ├── agent_gateway.rs
    │       └── agent_persistence.rs
    └── agent/
        ├── src/
        │   ├── main.rs
        │   ├── lib.rs
        │   ├── config.rs
        │   ├── runtime.rs
        │   ├── emulator/
        │   │   ├── mod.rs
        │   │   ├── driver.rs
        │   │   └── fake.rs
        │   └── ws/
        │       ├── mod.rs
        │       └── client.rs
        └── tests/
            └── reconnect.rs
```

---

### Task 1: Define the versioned protocol

**Files:**
- Modify: `foster-platform/Cargo.toml`
- Modify: `foster-platform/crates/protocol/Cargo.toml`
- Modify: `foster-platform/crates/protocol/src/lib.rs`
- Create: `foster-platform/crates/protocol/src/envelope.rs`
- Create: `foster-platform/crates/protocol/src/server.rs`
- Create: `foster-platform/crates/protocol/src/agent.rs`

**Interfaces:**
- Produces `PROTOCOL_VERSION: u16 = 1`.
- Produces `ServerEnvelope`, `AgentEnvelope`, `ServerCommand`, `AgentEvent`.
- Produces `validate_protocol_version(received: u16) -> Result<(), ProtocolVersionError>`.

- [ ] **Step 1: Add shared dependencies**

Add workspace dependencies:

```toml
dashmap = "6"
futures-util = "0.3"
http = "1"
serde_json = "1"
tokio-tungstenite = "0.28"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
uuid = { version = "1", features = ["v4", "serde"] }
axum = { version = "0.8", features = ["ws"] }
async-trait = "0.1"
```

Protocol dependencies gain:

```toml
chrono.workspace = true
serde_json.workspace = true
thiserror.workspace = true
uuid.workspace = true
```

- [ ] **Step 2: Write failing protocol tests**

Create `envelope.rs` with tests that reference yet-undefined production types:

```rust
#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    use crate::{
        validate_protocol_version, AgentEnvelope, AgentEvent, AgentHello,
        ProtocolVersionError, PROTOCOL_VERSION,
    };

    #[test]
    fn hello_serializes_with_stable_type_name() {
        let envelope = AgentEnvelope {
            protocol_version: PROTOCOL_VERSION,
            event_id: Uuid::nil(),
            sent_at: DateTime::<Utc>::from_timestamp(0, 0).unwrap(),
            payload: AgentEvent::Hello(AgentHello {
                agent_id: "agent-01".into(),
                host_id: 7,
                agent_version: "0.1.0".into(),
                hostname: "win-host".into(),
                os_version: "windows".into(),
                capabilities: vec!["EMULATOR_DISCOVERY".into()],
            }),
        };

        let value = serde_json::to_value(envelope).unwrap();

        assert_eq!(value["protocol_version"], 1);
        assert_eq!(value["payload"]["type"], "HELLO");
        assert_eq!(value["payload"]["data"]["host_id"], 7);
    }

    #[test]
    fn unsupported_protocol_version_is_rejected() {
        assert_eq!(
            validate_protocol_version(PROTOCOL_VERSION + 1),
            Err(ProtocolVersionError {
                received: PROTOCOL_VERSION + 1,
                expected: PROTOCOL_VERSION,
            })
        );
    }
}
```

Run CI and require failure due unresolved protocol types/functions.

- [ ] **Step 3: Implement the protocol types**

`server.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingCommand {
    pub nonce: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RefreshEmulatorsCommand {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServerCommand {
    Ping(PingCommand),
    RefreshEmulators(RefreshEmulatorsCommand),
}
```

`agent.rs`:

```rust
use foster_domain::EmulatorStatus;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHello {
    pub agent_id: String,
    pub host_id: i64,
    pub agent_version: String,
    pub hostname: String,
    pub os_version: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmulatorHeartbeat {
    pub emulator_code: String,
    pub status: EmulatorStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heartbeat {
    pub host_id: i64,
    pub emulators: Vec<EmulatorHeartbeat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmulatorDescriptor {
    pub emulator_code: String,
    pub driver_type: String,
    pub adb_serial: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmulatorSnapshot {
    pub host_id: i64,
    pub emulators: Vec<EmulatorDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pong {
    pub nonce: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentEvent {
    Hello(AgentHello),
    Heartbeat(Heartbeat),
    EmulatorSnapshot(EmulatorSnapshot),
    Pong(Pong),
}
```

`envelope.rs` production types:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AgentEvent, ServerCommand};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerEnvelope {
    pub protocol_version: u16,
    pub command_id: Uuid,
    pub sent_at: DateTime<Utc>,
    pub payload: ServerCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEnvelope {
    pub protocol_version: u16,
    pub event_id: Uuid,
    pub sent_at: DateTime<Utc>,
    pub payload: AgentEvent,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("unsupported protocol version {received}; expected {expected}")]
pub struct ProtocolVersionError {
    pub received: u16,
    pub expected: u16,
}

pub fn validate_protocol_version(
    received: u16,
) -> Result<(), ProtocolVersionError> {
    if received == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ProtocolVersionError {
            received,
            expected: PROTOCOL_VERSION,
        })
    }
}
```

Export all modules from `lib.rs`.

- [ ] **Step 4: Verify and commit**

CI must pass `fmt/check/test/clippy`.

Commit:

`feat: define foster agent websocket protocol`

---

### Task 2: Build Server app, Agent authentication, connection registry, HELLO and timeout behavior

**Files:**
- Modify: `foster-platform/crates/server/Cargo.toml`
- Modify: `foster-platform/crates/server/src/lib.rs`
- Modify: `foster-platform/crates/server/src/main.rs`
- Create: `foster-platform/crates/server/src/config.rs`
- Create: `foster-platform/crates/server/src/app.rs`
- Create: `foster-platform/crates/server/src/agent_gateway/mod.rs`
- Create: `foster-platform/crates/server/src/agent_gateway/auth.rs`
- Create: `foster-platform/crates/server/src/agent_gateway/registry.rs`
- Create: `foster-platform/crates/server/src/agent_gateway/handler.rs`
- Create: `foster-platform/crates/server/tests/agent_gateway.rs`

**Interfaces:**
- HTTP `GET /healthz` returns 200 JSON `{"status":"ok"}`.
- WebSocket endpoint: `GET /agent/ws`.
- Authorization: `Authorization: Bearer <FOSTER_AGENT_TOKEN>`.
- `AgentRegistry::is_online(host_id)`.
- `AgentRegistry::connection_count()`.
- Each presence has a unique `connection_id: Uuid`.

- [ ] **Step 1: Write failing HTTP/WebSocket tests**

Tests:
- `healthz_returns_ok`
- `missing_agent_token_is_rejected`
- `invalid_agent_token_is_rejected`
- `valid_hello_registers_known_host`
- `unsupported_protocol_version_never_registers_host`
- `replacement_connection_survives_old_socket_close`
- `host_becomes_offline_after_heartbeat_timeout`

Use a test gateway config:

```rust
AgentGatewayConfig {
    agent_token: "test-token".into(),
    heartbeat_timeout: Duration::from_millis(150),
    sweep_interval: Duration::from_millis(25),
}
```

The replacement-socket test must:
1. connect socket A and send valid HELLO for host 7;
2. connect socket B and send valid HELLO for host 7;
3. close socket A;
4. assert registry still reports host 7 online and connection count is 1.

- [ ] **Step 2: Implement config**

```rust
#[derive(Debug, Clone)]
pub struct AgentGatewayConfig {
    pub agent_token: String,
    pub heartbeat_timeout: Duration,
    pub sweep_interval: Duration,
}

impl AgentGatewayConfig {
    pub fn production(agent_token: String) -> Self {
        Self {
            agent_token,
            heartbeat_timeout: Duration::from_secs(45),
            sweep_interval: Duration::from_secs(5),
        }
    }
}
```

- [ ] **Step 3: Implement connection registry**

```rust
#[derive(Debug, Clone)]
pub struct AgentPresence {
    pub connection_id: Uuid,
    pub agent_id: String,
    pub host_id: i64,
    pub connected_at: Instant,
    pub last_heartbeat_at: Instant,
}

#[derive(Clone, Default)]
pub struct AgentRegistry {
    inner: Arc<DashMap<i64, AgentPresence>>,
}
```

Required methods:

```rust
pub fn register(&self, presence: AgentPresence);
pub fn heartbeat(&self, host_id: i64, connection_id: Uuid) -> bool;
pub fn remove_if_current(&self, host_id: i64, connection_id: Uuid) -> bool;
pub fn is_online(&self, host_id: i64) -> bool;
pub fn connection_count(&self) -> usize;
pub fn remove_stale(&self, timeout: Duration) -> Vec<i64>;
```

`remove_if_current` must compare connection ids before removal.

- [ ] **Step 4: Implement auth, app, and HELLO handler**

First valid WebSocket frame must decode as `AgentEnvelope::Hello`.

Before registration:
1. validate Bearer token;
2. validate protocol version;
3. validate Host exists in MySQL;
4. create connection id;
5. register presence;
6. mark Host ONLINE.

Heartbeat must only update the connection that owns the current connection id.

Socket close calls `remove_if_current`; only when it actually removes the current presence may the handler mark Host OFFLINE.

- [ ] **Step 5: Implement timeout sweeper**

Every `sweep_interval`:
- remove stale current presence records;
- persist each removed host as OFFLINE.

Production timeout is 45 seconds.

- [ ] **Step 6: Verify and commit**

CI must pass.

Commit:

`feat: add authenticated agent websocket gateway`

---

### Task 3: Persist HELLO, heartbeat and emulator snapshots

**Files:**
- Modify: `foster-platform/crates/server/src/control_plane/repository.rs`
- Modify: `foster-platform/crates/server/src/agent_gateway/handler.rs`
- Create: `foster-platform/crates/server/tests/agent_persistence.rs`

**Interfaces:**
- `host_exists(pool, host_id)`
- `mark_host_online(pool, host_id, agent_version)`
- `touch_host_heartbeat(pool, host_id)`
- `mark_host_offline(pool, host_id)`
- `upsert_emulator_snapshot(pool, host_id, items)`

- [ ] **Step 1: Write failing persistence tests**

Required tests:
- `hello_updates_existing_host_online`
- `hello_for_unknown_host_is_rejected_without_insert`
- `heartbeat_updates_last_heartbeat_at`
- `emulator_snapshot_inserts_new_emulator`
- `emulator_snapshot_preserves_server_controlled_fields`

For preserve-fields:
1. seed emulator with `max_account_count = 9` and `current_job_id = 'job-1'`;
2. send snapshot with same emulator code but changed driver/ADB;
3. assert capacity still 9 and current job still `job-1`.

- [ ] **Step 2: Implement repository methods**

`upsert_emulator_snapshot` uses:

```sql
INSERT INTO emulator_instance(
    host_id, emulator_code, driver_type, adb_serial,
    status, last_heartbeat_at
)
VALUES (?, ?, ?, ?, 'IDLE', NOW(3))
ON DUPLICATE KEY UPDATE
    host_id = VALUES(host_id),
    driver_type = VALUES(driver_type),
    adb_serial = VALUES(adb_serial),
    last_heartbeat_at = VALUES(last_heartbeat_at)
```

It must not update:
- `max_account_count`
- `current_job_id`

- [ ] **Step 3: Wire event persistence**

HELLO:
- reject unknown Host;
- update `status='ONLINE'`;
- update `agent_version`;
- set `last_heartbeat_at=NOW(3)`.

Heartbeat:
- ensure payload host id equals connected host id;
- touch Host heartbeat timestamp.

Snapshot:
- ensure payload host id equals connected host id;
- upsert descriptors.

- [ ] **Step 4: Verify and commit**

CI must pass.

Commit:

`feat: persist agent and emulator presence`

---

### Task 4: Implement EmulatorDriver abstraction and Fake driver

**Files:**
- Modify: `foster-platform/crates/agent/Cargo.toml`
- Create: `foster-platform/crates/agent/src/lib.rs`
- Create: `foster-platform/crates/agent/src/emulator/mod.rs`
- Create: `foster-platform/crates/agent/src/emulator/driver.rs`
- Create: `foster-platform/crates/agent/src/emulator/fake.rs`

**Interfaces:**
- `EmulatorDriver`
- `FakeEmulatorDriver::new(Vec<EmulatorDescriptor>)`

- [ ] **Step 1: Write failing fake-driver tests**

```rust
#[tokio::test]
async fn fake_driver_reports_configured_instances() {
    let driver = FakeEmulatorDriver::new(vec![
        EmulatorDescriptor {
            emulator_code: "emu-01".into(),
            driver_type: "FAKE".into(),
            adb_serial: Some("127.0.0.1:5555".into()),
        },
    ]);

    let instances = driver.list_instances().await.unwrap();

    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].emulator_code, "emu-01");
}

#[tokio::test]
async fn fake_driver_rejects_unknown_instance() {
    let driver = FakeEmulatorDriver::new(Vec::new());

    let result = driver.start("missing").await;

    assert!(matches!(
        result,
        Err(EmulatorDriverError::UnknownInstance(value))
            if value == "missing"
    ));
}
```

- [ ] **Step 2: Implement trait**

```rust
#[derive(Debug, thiserror::Error)]
pub enum EmulatorDriverError {
    #[error("unknown emulator instance: {0}")]
    UnknownInstance(String),
    #[error("{0}")]
    Message(String),
}

#[async_trait]
pub trait EmulatorDriver: Send + Sync + 'static {
    async fn list_instances(
        &self,
    ) -> Result<Vec<EmulatorDescriptor>, EmulatorDriverError>;

    async fn start(&self, instance_id: &str) -> Result<(), EmulatorDriverError>;
    async fn stop(&self, instance_id: &str) -> Result<(), EmulatorDriverError>;
    async fn adb_serial(
        &self,
        instance_id: &str,
    ) -> Result<Option<String>, EmulatorDriverError>;
    async fn screenshot(
        &self,
        instance_id: &str,
    ) -> Result<Vec<u8>, EmulatorDriverError>;
}
```

Fake driver behavior:
- list returns configured descriptors;
- known start/stop succeeds;
- unknown start/stop/adb/screenshot returns `UnknownInstance`;
- known screenshot returns an empty byte vector in Phase 2.

- [ ] **Step 3: Verify and commit**

CI must pass.

Commit:

`feat: add emulator driver abstraction`

---

### Task 5: Implement Agent WebSocket runtime, heartbeat, refresh and reconnect

**Files:**
- Create: `foster-platform/crates/agent/src/config.rs`
- Create: `foster-platform/crates/agent/src/runtime.rs`
- Create: `foster-platform/crates/agent/src/ws/mod.rs`
- Create: `foster-platform/crates/agent/src/ws/client.rs`
- Modify: `foster-platform/crates/agent/src/lib.rs`
- Modify: `foster-platform/crates/agent/src/main.rs`
- Create: `foster-platform/crates/agent/tests/reconnect.rs`

**Interfaces:**
- `AgentConfig`
- `AgentRuntime<D: EmulatorDriver>`
- outbound Bearer-authenticated WebSocket client.

- [ ] **Step 1: Write failing reconnect/runtime tests**

Required tests:
- `agent_sends_hello_after_connect`
- `agent_sends_snapshot_after_hello`
- `agent_sends_heartbeat_on_interval`
- `refresh_emulators_sends_fresh_snapshot`
- `agent_reconnects_and_sends_hello_again`

Reconnect test server:
1. accept authenticated socket;
2. receive HELLO;
3. close it;
4. accept reconnect;
5. receive second HELLO;
6. assert same `agent_id` and `host_id`.

Use:

```rust
AgentConfig {
    server_ws_url,
    agent_token: "test-token".into(),
    agent_id: "agent-01".into(),
    host_id: 7,
    heartbeat_interval: Duration::from_millis(50),
    reconnect_delays: vec![
        Duration::from_millis(10),
        Duration::from_millis(20),
        Duration::from_millis(30),
    ],
}
```

- [ ] **Step 2: Implement Agent config**

Production config:

```rust
impl AgentConfig {
    pub fn production(
        server_ws_url: String,
        agent_token: String,
        agent_id: String,
        host_id: i64,
    ) -> Self {
        Self {
            server_ws_url,
            agent_token,
            agent_id,
            host_id,
            heartbeat_interval: Duration::from_secs(15),
            reconnect_delays: vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(16),
                Duration::from_secs(30),
            ],
        }
    }
}
```

- [ ] **Step 3: Implement authenticated WS client**

Every connection request includes:

`Authorization: Bearer <agent_token>`

Return a typed WebSocket stream; do not log the token.

- [ ] **Step 4: Implement AgentRuntime**

Per connection:
1. connect;
2. send HELLO;
3. call `driver.list_instances()`;
4. send EmulatorSnapshot;
5. start heartbeat ticker;
6. read ServerCommand;
7. Ping → Pong with same nonce;
8. RefreshEmulators → call driver again and send a fresh snapshot;
9. disconnect → reconnect using configured delays;
10. once a connection survives at least one heartbeat interval, reset backoff to the first delay.

Every envelope uses `PROTOCOL_VERSION`, new UUID, and `Utc::now()`.

- [ ] **Step 5: Make Agent main runnable**

Read:
- `FOSTER_SERVER_WS_URL`
- `FOSTER_AGENT_TOKEN`
- `FOSTER_AGENT_ID`
- `FOSTER_HOST_ID`

Instantiate `FakeEmulatorDriver::new(Vec::new())`.

Log that Phase 2 uses the fake driver; do not claim real emulator support.

- [ ] **Step 6: Verify and commit**

CI must pass.

Commit:

`feat: add reconnecting foster agent runtime`

---

## Phase 2 Completion Gate

Do not begin QR Login until all are true:

- Protocol JSON uses stable uppercase type tags and version 1.
- Unsupported protocol version does not register a Host.
- Missing/invalid Bearer token is rejected.
- Unknown Host is not auto-created.
- Valid HELLO marks an existing Host ONLINE.
- Heartbeat advances `last_heartbeat_at`.
- Timeout marks Host OFFLINE.
- Old socket close cannot remove a newer connection for the same Host.
- Emulator snapshot inserts/updates Agent-observed fields.
- Emulator snapshot preserves `max_account_count` and `current_job_id`.
- FakeEmulatorDriver tests pass.
- Agent sends HELLO and snapshot.
- Agent sends heartbeat at configured interval.
- RefreshEmulators produces a fresh snapshot.
- Agent reconnects and sends HELLO again.
- Full `cargo fmt --all --check`, `cargo check --workspace`, `cargo test --workspace --all-targets`, and `cargo clippy --workspace --all-targets -- -D warnings` pass in CI.
