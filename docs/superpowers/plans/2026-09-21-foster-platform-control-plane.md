# Foster Platform Control Plane Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first independently testable slice of the foster platform: a Rust workspace with core domain rules, MySQL persistence for hosts/emulators/accounts/bindings, account identity verification, and a working Server ↔ Agent WebSocket control plane.

**Architecture:** Add an isolated `foster-platform/` Rust workspace to the existing OAS fork without restructuring Python OAS. The Rust Server is the authority for host/emulator/account/binding state; the Rust Agent connects outbound over WebSocket and exposes an emulator-driver abstraction. This phase deliberately stops before QR login, foster scheduling, resource-pool allocation, H5, and OAS execution so the control-plane foundation can be reviewed and tested independently.

**Tech Stack:** Rust stable, Tokio, Axum, SQLx + MySQL 8, Redis client dependency reserved for later phases but no Redis runtime behavior in this phase, Serde, UUID, Chrono, Tracing, async-trait, tokio-tungstenite, Docker Compose for local MySQL.

**Spec:** `docs/superpowers/specs/2026-09-21-onmyoji-foster-platform-design.md`

## Global Constraints

- Work only on branch `feat/foster-platform`; keep `master` suitable for upstream fork synchronization.
- Keep existing Python OAS behavior intact in this phase; do not modify `tasks/KekkaiUtilize` or `tasks/Component/SwitchAccount`.
- Rust Server is the business-state authority; Agent state is observational and must not override Server state.
- One emulator may hold multiple game accounts and must enforce configurable `max_account_count`.
- One `GameAccount` may have at most one ACTIVE emulator binding at any instant.
- Capacity counts bindings in `PENDING`, `ACTIVE`, and `MIGRATING` states.
- Masked account text is never sufficient by itself to verify an account.
- Agent initiates the connection to Server over WebSocket; production heartbeat interval is 15 seconds and offline timeout is 45 seconds.
- Default business timezone remains `Asia/Shanghai`.
- Server and Agent protocol messages use explicit protocol versioning and stable serialized names.
- No payment, QR login, foster scheduler, platform resource allocation, H5, emulator-vendor driver, or OAS execution is implemented in this phase.
- Use TDD for every behavior task: failing test, confirm failure, minimal implementation, confirm pass, then commit.

## Review Focus

- Two different accounts can share the same masked login text; identity verification must remain AMBIGUOUS unless a stronger independent factor matches. Covered by Task 2 tests.
- Two concurrent allocations racing for the last emulator slot must result in exactly one reservation, never capacity overflow. Covered by Task 4 MySQL concurrency test.
- A game account with an existing ACTIVE or reserved binding must not receive another ordinary PENDING binding. Covered by Task 4 integration tests and Task 3 database constraints.
- Invalid Agent token or unsupported protocol version must not create an online Agent connection. Covered by Task 6 WebSocket integration tests.
- A dropped Agent connection must reconnect and re-send HELLO without creating duplicate logical connections. Covered by Task 7 reconnect test.

---

## File Structure

This phase creates the following focused structure:

```text
foster-platform/
├── Cargo.toml                         # workspace membership and shared dependencies
├── rust-toolchain.toml                # stable toolchain declaration
├── .env.example                       # local Server/Agent/MySQL settings
├── docker-compose.dev.yml             # MySQL 8 local test/development service
├── README.md                          # run/test instructions for this phase
├── migrations/
│   └── 0001_control_plane.sql         # host/emulator/account/identity/binding schema
└── crates/
    ├── domain/
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── account.rs
    │       ├── binding.rs
    │       ├── emulator.rs
    │       ├── host.rs
    │       └── identity.rs
    ├── protocol/
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── agent.rs
    │       ├── envelope.rs
    │       └── server.rs
    ├── server/
    │   ├── Cargo.toml
    │   ├── src/
    │   │   ├── main.rs
    │   │   ├── app.rs
    │   │   ├── config.rs
    │   │   ├── agent_gateway/
    │   │   │   ├── mod.rs
    │   │   │   ├── auth.rs
    │   │   │   ├── handler.rs
    │   │   │   └── registry.rs
    │   │   └── control_plane/
    │   │       ├── mod.rs
    │   │       ├── allocator.rs
    │   │       └── repository.rs
    │   └── tests/
    │       ├── agent_gateway.rs
    │       └── binding_allocator.rs
    └── agent/
        ├── Cargo.toml
        ├── src/
        │   ├── main.rs
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

The workspace lives under `foster-platform/` so the Python OAS root remains recognizable and easy to sync from upstream.

---

### Task 1: Create the Rust workspace and compile-only baseline

**Files:**
- Create: `foster-platform/Cargo.toml`
- Create: `foster-platform/rust-toolchain.toml`
- Create: `foster-platform/crates/domain/Cargo.toml`
- Create: `foster-platform/crates/domain/src/lib.rs`
- Create: `foster-platform/crates/protocol/Cargo.toml`
- Create: `foster-platform/crates/protocol/src/lib.rs`
- Create: `foster-platform/crates/server/Cargo.toml`
- Create: `foster-platform/crates/server/src/main.rs`
- Create: `foster-platform/crates/agent/Cargo.toml`
- Create: `foster-platform/crates/agent/src/main.rs`

**Interfaces:**
- Consumes: none.
- Produces: four workspace packages named `foster-domain`, `foster-protocol`, `foster-server`, and `foster-agent`.

- [ ] **Step 1: Write a workspace metadata smoke check that initially fails because the workspace does not exist**

Run:

```bash
cd foster-platform
cargo metadata --no-deps --format-version 1
```

Expected before creating files: FAIL because `foster-platform/Cargo.toml` does not exist.

- [ ] **Step 2: Create the workspace manifest**

Create `foster-platform/Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
    "crates/domain",
    "crates/protocol",
    "crates/server",
    "crates/agent",
]

[workspace.package]
edition = "2024"

[workspace.dependencies]
async-trait = "0.1"
axum = { version = "0.8", features = ["ws"] }
chrono = { version = "0.4", features = ["serde"] }
dashmap = "6"
futures-util = "0.3"
http = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "mysql", "chrono", "uuid", "migrate"] }
thiserror = "2"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "signal", "sync", "time", "test-util"] }
tokio-tungstenite = "0.28"
tower = { version = "0.5", features = ["util"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
uuid = { version = "1", features = ["v4", "serde"] }
```

Create `foster-platform/rust-toolchain.toml`:

```toml
[toolchain]
channel = "stable"
profile = "minimal"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 3: Create minimal package manifests and entrypoints**

`crates/domain/Cargo.toml`:

```toml
[package]
name = "foster-domain"
version = "0.1.0"
edition.workspace = true

[dependencies]
chrono.workspace = true
serde.workspace = true
thiserror.workspace = true
```

`crates/domain/src/lib.rs`:

```rust
pub const DOMAIN_VERSION: &str = env!("CARGO_PKG_VERSION");
```

`crates/protocol/Cargo.toml`:

```toml
[package]
name = "foster-protocol"
version = "0.1.0"
edition.workspace = true

[dependencies]
chrono.workspace = true
foster-domain = { path = "../domain" }
serde.workspace = true
uuid.workspace = true
```

`crates/protocol/src/lib.rs`:

```rust
pub const PROTOCOL_VERSION: u16 = 1;
```

`crates/server/Cargo.toml`:

```toml
[package]
name = "foster-server"
version = "0.1.0"
edition.workspace = true

[dependencies]
axum.workspace = true
chrono.workspace = true
dashmap.workspace = true
foster-domain = { path = "../domain" }
foster-protocol = { path = "../protocol" }
futures-util.workspace = true
http.workspace = true
serde.workspace = true
serde_json.workspace = true
sqlx.workspace = true
thiserror.workspace = true
tokio.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
uuid.workspace = true

[dev-dependencies]
tokio-tungstenite.workspace = true
tower.workspace = true
```

`crates/server/src/main.rs`:

```rust
fn main() {
    println!("foster-server");
}
```

`crates/agent/Cargo.toml`:

```toml
[package]
name = "foster-agent"
version = "0.1.0"
edition.workspace = true

[dependencies]
async-trait.workspace = true
chrono.workspace = true
foster-domain = { path = "../domain" }
foster-protocol = { path = "../protocol" }
futures-util.workspace = true
http.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
tokio-tungstenite.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
uuid.workspace = true
```

`crates/agent/src/main.rs`:

```rust
fn main() {
    println!("foster-agent");
}
```

- [ ] **Step 4: Format, compile, and lint the workspace**

Run:

```bash
cd foster-platform
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: all commands PASS.

- [ ] **Step 5: Commit**

```bash
git add foster-platform
git commit -m "build: add foster platform rust workspace"
```

---

### Task 2: Implement domain models and deterministic account identity verification

**Files:**
- Create: `foster-platform/crates/domain/src/host.rs`
- Create: `foster-platform/crates/domain/src/emulator.rs`
- Create: `foster-platform/crates/domain/src/account.rs`
- Create: `foster-platform/crates/domain/src/binding.rs`
- Create: `foster-platform/crates/domain/src/identity.rs`
- Modify: `foster-platform/crates/domain/src/lib.rs`

**Interfaces:**
- Consumes: no external service.
- Produces:
  - `HostStatus`
  - `EmulatorStatus`
  - `BindingStatus`
  - `IdentityType`
  - `AccountIdentity`
  - `DetectedIdentity`
  - `IdentityDecision`
  - `verify_identity(stored: &[AccountIdentity], detected: &DetectedIdentity) -> IdentityDecision`

- [ ] **Step 1: Write failing identity-verification tests**

Add tests at the bottom of `identity.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn identity(kind: IdentityType, value: &str) -> AccountIdentity {
        AccountIdentity {
            kind,
            value: value.to_string(),
            normalized_value: normalize_identity(kind, value),
            confidence: 100,
        }
    }

    #[test]
    fn masked_account_alone_is_ambiguous() {
        let stored = vec![identity(IdentityType::MaskedAccount, "138****5678")];
        let detected = DetectedIdentity {
            masked_account: Some("138****5678".into()),
            character_name: None,
            server_name: None,
            game_uid: None,
            ocr_aliases: vec![],
        };

        assert_eq!(
            verify_identity(&stored, &detected),
            IdentityDecision::Ambiguous
        );
    }

    #[test]
    fn masked_account_plus_character_verifies() {
        let stored = vec![
            identity(IdentityType::MaskedAccount, "138****5678"),
            identity(IdentityType::CharacterName, "柳某某"),
        ];
        let detected = DetectedIdentity {
            masked_account: Some("138****5678".into()),
            character_name: Some("柳某某".into()),
            server_name: None,
            game_uid: None,
            ocr_aliases: vec![],
        };

        assert!(matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Verified { .. }
        ));
    }

    #[test]
    fn character_server_pair_verifies_without_masked_account() {
        let stored = vec![
            identity(IdentityType::CharacterName, "柳某某"),
            identity(IdentityType::ServerName, "春之樱"),
        ];
        let detected = DetectedIdentity {
            masked_account: None,
            character_name: Some("柳某某".into()),
            server_name: Some("春之樱".into()),
            game_uid: None,
            ocr_aliases: vec![],
        };

        assert!(matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Verified { .. }
        ));
    }

    #[test]
    fn uid_conflict_is_mismatch_even_when_mask_matches() {
        let stored = vec![
            identity(IdentityType::MaskedAccount, "138****5678"),
            identity(IdentityType::GameUid, "10001"),
        ];
        let detected = DetectedIdentity {
            masked_account: Some("138****5678".into()),
            character_name: None,
            server_name: None,
            game_uid: Some("99999".into()),
            ocr_aliases: vec![],
        };

        assert!(matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Mismatch { .. }
        ));
    }

    #[test]
    fn same_masked_text_for_two_people_does_not_become_identity_proof() {
        let stored = vec![identity(IdentityType::MaskedAccount, "138****5678")];
        let detected = DetectedIdentity {
            masked_account: Some("138****5678".into()),
            character_name: Some("另一个角色".into()),
            server_name: Some("另一个区服".into()),
            game_uid: None,
            ocr_aliases: vec![],
        };

        assert_eq!(
            verify_identity(&stored, &detected),
            IdentityDecision::Ambiguous
        );
    }
}
```

- [ ] **Step 2: Run tests and confirm failure**

Run:

```bash
cd foster-platform
cargo test -p foster-domain identity::tests -- --nocapture
```

Expected: FAIL because the domain types and `verify_identity` do not exist.

- [ ] **Step 3: Implement the domain types**

`host.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HostStatus {
    Online,
    Offline,
    Maintenance,
}
```

`emulator.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EmulatorStatus {
    Offline,
    Idle,
    SwitchingAccount,
    Running,
    LoginSession,
    Maintenance,
    Error,
}
```

`binding.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BindingStatus {
    Pending,
    Active,
    Migrating,
    Unbound,
}
```

`account.rs`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LoginStatus {
    Pending,
    LoggedIn,
    LoginExpired,
    ReloginRequired,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VerifyStatus {
    Pending,
    Verified,
    Mismatch,
    Ambiguous,
}
```

Implement `identity.rs` with these public shapes:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IdentityType {
    MaskedAccount,
    OcrAlias,
    CharacterName,
    ServerName,
    GameUid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountIdentity {
    pub kind: IdentityType,
    pub value: String,
    pub normalized_value: String,
    pub confidence: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DetectedIdentity {
    pub masked_account: Option<String>,
    pub character_name: Option<String>,
    pub server_name: Option<String>,
    pub game_uid: Option<String>,
    pub ocr_aliases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityDecision {
    Verified { matched: Vec<IdentityType> },
    Ambiguous,
    Mismatch { conflicting: Vec<IdentityType> },
}

pub fn normalize_identity(kind: IdentityType, value: &str) -> String {
    match kind {
        IdentityType::MaskedAccount | IdentityType::OcrAlias => {
            value.trim().to_ascii_lowercase().replace(' ', "")
        }
        _ => value.trim().to_string(),
    }
}

pub fn verify_identity(
    stored: &[AccountIdentity],
    detected: &DetectedIdentity,
) -> IdentityDecision {
    // Build normalized stored values by IdentityType.
    // A conflicting GameUid is always Mismatch.
    // If both stored and detected CharacterName exist and differ, record CharacterName conflict.
    // If both stored and detected ServerName exist and differ, record ServerName conflict.
    // Any recorded strong conflict returns Mismatch.
    // A matching GameUid returns Verified.
    // Matching CharacterName + ServerName returns Verified.
    // Matching MaskedAccount/OcrAlias plus either CharacterName or ServerName returns Verified.
    // MaskedAccount/OcrAlias alone returns Ambiguous.
    // No sufficient evidence returns Ambiguous.
    unimplemented!()
}
```

Replace the `unimplemented!()` immediately in the same edit with the minimal implementation needed by the five tests; do not leave the macro in committed code.

Update `lib.rs`:

```rust
pub mod account;
pub mod binding;
pub mod emulator;
pub mod host;
pub mod identity;

pub use account::*;
pub use binding::*;
pub use emulator::*;
pub use host::*;
pub use identity::*;
```

- [ ] **Step 4: Run tests and lint**

Run:

```bash
cd foster-platform
cargo test -p foster-domain
cargo clippy -p foster-domain --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add foster-platform/crates/domain
git commit -m "feat: add foster control plane domain rules"
```

---

### Task 3: Add the MySQL control-plane schema with database-enforced binding invariants

**Files:**
- Create: `foster-platform/migrations/0001_control_plane.sql`
- Create: `foster-platform/docker-compose.dev.yml`
- Create: `foster-platform/.env.example`
- Modify: `foster-platform/crates/server/Cargo.toml`
- Create: `foster-platform/crates/server/tests/schema_invariants.rs`

**Interfaces:**
- Consumes: domain enum string values from Task 2.
- Produces MySQL tables:
  - `host`
  - `emulator_instance`
  - `game_account`
  - `game_account_identity`
  - `emulator_account_binding`

- [ ] **Step 1: Write failing schema invariant tests**

Create `crates/server/tests/schema_invariants.rs`:

```rust
use sqlx::MySqlPool;

#[sqlx::test(migrations = "../../migrations")]
async fn database_rejects_two_active_bindings_for_same_account(
    pool: MySqlPool,
) -> sqlx::Result<()> {
    let host_id = insert_host(&pool).await?;
    let emu_a = insert_emulator(&pool, host_id, "emu-a", 5).await?;
    let emu_b = insert_emulator(&pool, host_id, "emu-b", 5).await?;
    let account_id = insert_account(&pool).await?;

    insert_binding(&pool, emu_a, account_id, 1, "ACTIVE").await?;

    let second = insert_binding(&pool, emu_b, account_id, 1, "ACTIVE").await;
    assert!(second.is_err());

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn database_rejects_two_occupied_bindings_for_same_emulator_slot(
    pool: MySqlPool,
) -> sqlx::Result<()> {
    let host_id = insert_host(&pool).await?;
    let emulator_id = insert_emulator(&pool, host_id, "emu-a", 5).await?;
    let first_account = insert_account(&pool).await?;
    let second_account = insert_account(&pool).await?;

    insert_binding(&pool, emulator_id, first_account, 1, "PENDING").await?;

    let second =
        insert_binding(&pool, emulator_id, second_account, 1, "PENDING").await;
    assert!(second.is_err());

    Ok(())
}
```

In the same test file add concrete helper functions using `sqlx::query` and `last_insert_id()`; helpers insert only the columns required by the migration.

- [ ] **Step 2: Run the test and confirm failure**

Start MySQL:

```bash
cd foster-platform
docker compose -f docker-compose.dev.yml up -d mysql
```

Before creating the compose/migration files this command or the subsequent test must fail.

After the container exists, use:

```bash
export DATABASE_URL='mysql://root:foster@127.0.0.1:3307/mysql'
cargo test -p foster-server --test schema_invariants -- --nocapture
```

Expected before migration implementation: FAIL because required tables do not exist.

- [ ] **Step 3: Create local MySQL configuration**

Create `docker-compose.dev.yml`:

```yaml
services:
  mysql:
    image: mysql:8.0
    environment:
      MYSQL_ROOT_PASSWORD: foster
      MYSQL_DATABASE: foster
    ports:
      - "3307:3306"
    command:
      - --character-set-server=utf8mb4
      - --collation-server=utf8mb4_0900_ai_ci
    healthcheck:
      test: ["CMD", "mysqladmin", "ping", "-hfoster", "-uroot", "-pfoster"]
      interval: 3s
      timeout: 3s
      retries: 20
```

Create `.env.example`:

```dotenv
FOSTER_BIND_ADDR=127.0.0.1:18080
FOSTER_DATABASE_URL=mysql://root:foster@127.0.0.1:3307/foster
FOSTER_AGENT_TOKEN=change-me
FOSTER_HOST_ID=1
FOSTER_AGENT_ID=dev-agent-01
FOSTER_SERVER_WS_URL=ws://127.0.0.1:18080/agent/ws
```

- [ ] **Step 4: Create the migration**

Use this schema in `migrations/0001_control_plane.sql`:

```sql
CREATE TABLE host (
    id BIGINT NOT NULL AUTO_INCREMENT,
    host_code VARCHAR(64) NOT NULL,
    hostname VARCHAR(128) NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'OFFLINE',
    agent_version VARCHAR(64) NULL,
    last_heartbeat_at DATETIME(3) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),
    PRIMARY KEY (id),
    UNIQUE KEY uk_host_code (host_code)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE emulator_instance (
    id BIGINT NOT NULL AUTO_INCREMENT,
    host_id BIGINT NOT NULL,
    emulator_code VARCHAR(64) NOT NULL,
    driver_type VARCHAR(32) NOT NULL DEFAULT 'UNKNOWN',
    max_account_count INT NOT NULL DEFAULT 5,
    status VARCHAR(32) NOT NULL DEFAULT 'OFFLINE',
    adb_serial VARCHAR(128) NULL,
    current_job_id VARCHAR(64) NULL,
    last_heartbeat_at DATETIME(3) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),
    PRIMARY KEY (id),
    UNIQUE KEY uk_emulator_code (emulator_code),
    KEY idx_emulator_host_status (host_id, status),
    CONSTRAINT fk_emulator_host
        FOREIGN KEY (host_id) REFERENCES host(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE game_account (
    id BIGINT NOT NULL AUTO_INCREMENT,
    customer_id BIGINT NOT NULL,
    character_name VARCHAR(64) NULL,
    server_name VARCHAR(64) NULL,
    game_uid VARCHAR(64) NULL,
    platform VARCHAR(32) NULL,
    login_status VARCHAR(32) NOT NULL DEFAULT 'PENDING',
    verify_status VARCHAR(32) NOT NULL DEFAULT 'PENDING',
    active_emulator_id BIGINT NULL,
    last_verified_at DATETIME(3) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),
    PRIMARY KEY (id),
    KEY idx_game_account_customer (customer_id),
    KEY idx_game_account_active_emulator (active_emulator_id),
    CONSTRAINT fk_game_account_active_emulator
        FOREIGN KEY (active_emulator_id) REFERENCES emulator_instance(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE game_account_identity (
    id BIGINT NOT NULL AUTO_INCREMENT,
    game_account_id BIGINT NOT NULL,
    identity_type VARCHAR(32) NOT NULL,
    identity_value VARCHAR(255) NOT NULL,
    normalized_value VARCHAR(255) NOT NULL,
    source VARCHAR(32) NOT NULL,
    confidence INT NOT NULL DEFAULT 100,
    enabled TINYINT(1) NOT NULL DEFAULT 1,
    last_seen_at DATETIME(3) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    PRIMARY KEY (id),
    KEY idx_identity_account (game_account_id),
    KEY idx_identity_lookup (identity_type, normalized_value, enabled),
    CONSTRAINT fk_identity_account
        FOREIGN KEY (game_account_id) REFERENCES game_account(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE emulator_account_binding (
    id BIGINT NOT NULL AUTO_INCREMENT,
    emulator_id BIGINT NOT NULL,
    game_account_id BIGINT NOT NULL,
    slot_no INT NOT NULL,
    status VARCHAR(32) NOT NULL,
    bound_at DATETIME(3) NULL,
    unbound_at DATETIME(3) NULL,
    created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
    updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3)
        ON UPDATE CURRENT_TIMESTAMP(3),

    active_account_guard BIGINT
        GENERATED ALWAYS AS (
            CASE WHEN status = 'ACTIVE' THEN game_account_id ELSE NULL END
        ) STORED,

    occupied_slot_guard VARCHAR(128)
        GENERATED ALWAYS AS (
            CASE
                WHEN status IN ('PENDING', 'ACTIVE', 'MIGRATING')
                THEN CONCAT(emulator_id, ':', slot_no)
                ELSE NULL
            END
        ) STORED,

    PRIMARY KEY (id),
    UNIQUE KEY uk_one_active_binding_per_account (active_account_guard),
    UNIQUE KEY uk_one_occupied_binding_per_slot (occupied_slot_guard),
    KEY idx_binding_account_status (game_account_id, status),
    KEY idx_binding_emulator_status (emulator_id, status),

    CONSTRAINT fk_binding_emulator
        FOREIGN KEY (emulator_id) REFERENCES emulator_instance(id),
    CONSTRAINT fk_binding_account
        FOREIGN KEY (game_account_id) REFERENCES game_account(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
```

- [ ] **Step 5: Run migrations and schema tests**

Run:

```bash
cd foster-platform
export DATABASE_URL='mysql://root:foster@127.0.0.1:3307/mysql'
cargo test -p foster-server --test schema_invariants -- --nocapture
```

Expected: both tests PASS.

- [ ] **Step 6: Commit**

```bash
git add foster-platform/migrations foster-platform/docker-compose.dev.yml foster-platform/.env.example foster-platform/crates/server
git commit -m "feat: add control plane database schema"
```

---

### Task 4: Implement transactional emulator capacity allocation

**Files:**
- Create: `foster-platform/crates/server/src/control_plane/mod.rs`
- Create: `foster-platform/crates/server/src/control_plane/repository.rs`
- Create: `foster-platform/crates/server/src/control_plane/allocator.rs`
- Create: `foster-platform/crates/server/tests/binding_allocator.rs`
- Modify: `foster-platform/crates/server/src/main.rs`

**Interfaces:**
- Consumes: MySQL schema from Task 3.
- Produces:
  - `BindingAllocator::new(MySqlPool) -> BindingAllocator`
  - `BindingAllocator::allocate_pending(game_account_id: i64) -> Result<AllocatedBinding, AllocationError>`
  - `AllocatedBinding { binding_id, emulator_id, slot_no }`
  - `AllocationError::{AlreadyBound, NoCapacity, AccountNotFound, Database}`

- [ ] **Step 1: Write failing allocator integration tests**

Create `tests/binding_allocator.rs` with these cases:

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn pending_binding_consumes_capacity(pool: MySqlPool) -> anyhow::Result<()> {
    let fixture = Fixture::single_emulator(&pool, 1).await?;
    let account_a = fixture.create_account(&pool).await?;
    let account_b = fixture.create_account(&pool).await?;

    let allocator = BindingAllocator::new(pool.clone());

    let first = allocator.allocate_pending(account_a).await?;
    assert_eq!(first.slot_no, 1);

    let second = allocator.allocate_pending(account_b).await;
    assert!(matches!(second, Err(AllocationError::NoCapacity)));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn same_account_cannot_receive_second_reserved_binding(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::two_emulators(&pool, 5).await?;
    let account = fixture.create_account(&pool).await?;

    let allocator = BindingAllocator::new(pool.clone());
    allocator.allocate_pending(account).await?;

    let second = allocator.allocate_pending(account).await;
    assert!(matches!(second, Err(AllocationError::AlreadyBound)));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_allocations_do_not_oversell_last_slot(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::single_emulator(&pool, 1).await?;
    let account_a = fixture.create_account(&pool).await?;
    let account_b = fixture.create_account(&pool).await?;

    let a = BindingAllocator::new(pool.clone());
    let b = BindingAllocator::new(pool.clone());

    let (left, right) = tokio::join!(
        a.allocate_pending(account_a),
        b.allocate_pending(account_b)
    );

    let successes = [left.is_ok(), right.is_ok()]
        .into_iter()
        .filter(|v| *v)
        .count();

    assert_eq!(successes, 1);

    Ok(())
}
```

Add `anyhow = "1"` only to `server` dev-dependencies for ergonomic fixture setup.

- [ ] **Step 2: Run tests and confirm failure**

Run:

```bash
cd foster-platform
export DATABASE_URL='mysql://root:foster@127.0.0.1:3307/mysql'
cargo test -p foster-server --test binding_allocator -- --nocapture
```

Expected: FAIL because `BindingAllocator` is undefined.

- [ ] **Step 3: Implement repository primitives**

In `repository.rs`, provide functions with these exact signatures:

```rust
pub async fn lock_game_account(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
) -> Result<GameAccountLockRow, sqlx::Error>;

pub async fn find_and_lock_candidate_emulator(
    tx: &mut Transaction<'_, MySql>,
) -> Result<Option<EmulatorCapacityRow>, sqlx::Error>;

pub async fn occupied_slots(
    tx: &mut Transaction<'_, MySql>,
    emulator_id: i64,
) -> Result<Vec<i32>, sqlx::Error>;

pub async fn insert_pending_binding(
    tx: &mut Transaction<'_, MySql>,
    emulator_id: i64,
    game_account_id: i64,
    slot_no: i32,
) -> Result<u64, sqlx::Error>;
```

Candidate emulator query requirements:

- eligible status: `IDLE`
- count `PENDING`, `ACTIVE`, and `MIGRATING`
- require occupied count < `max_account_count`
- order by occupied count ascending, then emulator id ascending
- lock selected emulator row with `FOR UPDATE SKIP LOCKED`

- [ ] **Step 4: Implement allocation transaction**

In `allocator.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllocatedBinding {
    pub binding_id: i64,
    pub emulator_id: i64,
    pub slot_no: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum AllocationError {
    #[error("game account not found")]
    AccountNotFound,
    #[error("game account already has a reserved or active binding")]
    AlreadyBound,
    #[error("no emulator capacity is available")]
    NoCapacity,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct BindingAllocator {
    pool: MySqlPool,
}

impl BindingAllocator {
    pub fn new(pool: MySqlPool) -> Self {
        Self { pool }
    }

    pub async fn allocate_pending(
        &self,
        game_account_id: i64,
    ) -> Result<AllocatedBinding, AllocationError> {
        // 1. begin transaction
        // 2. SELECT target game_account FOR UPDATE
        // 3. reject active_emulator_id != NULL
        // 4. reject any PENDING/ACTIVE/MIGRATING binding for this account
        // 5. lock one eligible emulator with remaining capacity
        // 6. read occupied slot numbers
        // 7. choose the lowest free slot in 1..=max_account_count
        // 8. insert PENDING binding
        // 9. commit and return AllocatedBinding
        unimplemented!()
    }
}
```

Replace `unimplemented!()` in the same edit with the complete minimal transaction implementation.

Important: if insertion loses a rare race due to the generated unique slot guard, roll back and return `NoCapacity`; never retry indefinitely inside the transaction.

- [ ] **Step 5: Run allocator and schema tests**

Run:

```bash
cd foster-platform
export DATABASE_URL='mysql://root:foster@127.0.0.1:3307/mysql'
cargo test -p foster-server --test binding_allocator -- --nocapture
cargo test -p foster-server --test schema_invariants -- --nocapture
cargo clippy -p foster-server --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add foster-platform/crates/server
git commit -m "feat: allocate emulator account capacity transactionally"
```

---

### Task 5: Define the versioned Server ↔ Agent protocol

**Files:**
- Create: `foster-platform/crates/protocol/src/envelope.rs`
- Create: `foster-platform/crates/protocol/src/server.rs`
- Create: `foster-platform/crates/protocol/src/agent.rs`
- Modify: `foster-platform/crates/protocol/src/lib.rs`

**Interfaces:**
- Consumes: `HostStatus`, `EmulatorStatus` from `foster-domain`.
- Produces:
  - `PROTOCOL_VERSION: u16 = 1`
  - `ServerEnvelope`
  - `AgentEnvelope`
  - `ServerCommand`
  - `AgentEvent`
  - `validate_protocol_version(u16) -> Result<(), ProtocolVersionError>`

- [ ] **Step 1: Write failing serialization tests**

Add tests in `envelope.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentEvent, AgentHello, PROTOCOL_VERSION};

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
        let error = validate_protocol_version(PROTOCOL_VERSION + 1).unwrap_err();
        assert_eq!(error.received, PROTOCOL_VERSION + 1);
    }
}
```

- [ ] **Step 2: Run tests and confirm failure**

```bash
cd foster-platform
cargo test -p foster-protocol -- --nocapture
```

Expected: FAIL because protocol types are undefined.

- [ ] **Step 3: Implement envelopes and protocol enums**

Use these shapes:

```rust
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
```

`ServerCommand` for Phase 1:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ServerCommand {
    Ping(PingCommand),
    RefreshEmulators(RefreshEmulatorsCommand),
}
```

`AgentEvent` for Phase 1:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AgentEvent {
    Hello(AgentHello),
    Heartbeat(Heartbeat),
    EmulatorSnapshot(EmulatorSnapshot),
    Pong(Pong),
}
```

Required Phase 1 payload structs:

```rust
pub struct AgentHello {
    pub agent_id: String,
    pub host_id: i64,
    pub agent_version: String,
    pub hostname: String,
    pub os_version: String,
    pub capabilities: Vec<String>,
}

pub struct Heartbeat {
    pub host_id: i64,
    pub emulators: Vec<EmulatorHeartbeat>,
}

pub struct EmulatorHeartbeat {
    pub emulator_code: String,
    pub status: EmulatorStatus,
}

pub struct EmulatorSnapshot {
    pub host_id: i64,
    pub emulators: Vec<EmulatorDescriptor>,
}

pub struct EmulatorDescriptor {
    pub emulator_code: String,
    pub driver_type: String,
    pub adb_serial: Option<String>,
}
```

Implement version validation:

```rust
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

Add `thiserror.workspace = true` and `serde_json.workspace = true` to protocol dependencies.

- [ ] **Step 4: Run tests and lint**

```bash
cd foster-platform
cargo test -p foster-protocol
cargo clippy -p foster-protocol --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add foster-platform/crates/protocol
git commit -m "feat: define foster agent websocket protocol"
```

---

### Task 6: Implement Server health endpoint, Agent authentication, registry, HELLO, and heartbeat timeout

**Files:**
- Create: `foster-platform/crates/server/src/config.rs`
- Create: `foster-platform/crates/server/src/app.rs`
- Create: `foster-platform/crates/server/src/agent_gateway/mod.rs`
- Create: `foster-platform/crates/server/src/agent_gateway/auth.rs`
- Create: `foster-platform/crates/server/src/agent_gateway/registry.rs`
- Create: `foster-platform/crates/server/src/agent_gateway/handler.rs`
- Modify: `foster-platform/crates/server/src/main.rs`
- Create: `foster-platform/crates/server/tests/agent_gateway.rs`

**Interfaces:**
- Consumes: Agent protocol from Task 5.
- Produces:
  - HTTP `GET /healthz` → `200 {"status":"ok"}`
  - WebSocket `GET /agent/ws`
  - Bearer-token Agent authentication
  - `AgentRegistry::is_online(host_id: i64) -> bool`
  - `AgentRegistry::connection_count() -> usize`
  - timeout-based removal of stale logical connections

- [ ] **Step 1: Write failing health and WebSocket tests**

Create `tests/agent_gateway.rs` with four cases:

1. `healthz_returns_ok`
2. `invalid_agent_token_is_rejected`
3. `hello_registers_host_online`
4. `unsupported_protocol_version_does_not_register_host`

For WebSocket integration tests, bind the Axum app to `127.0.0.1:0`, spawn `axum::serve`, and connect with `tokio_tungstenite::connect_async`.

Build the authenticated request exactly like:

```rust
let request = http::Request::builder()
    .uri(format!("ws://{addr}/agent/ws"))
    .header("Authorization", "Bearer test-token")
    .body(())
    .unwrap();
```

After sending HELLO, poll the shared registry for no more than one second and assert `is_online(host_id)`.

Use test gateway settings:

```rust
AgentGatewayConfig {
    agent_token: "test-token".into(),
    heartbeat_timeout: Duration::from_millis(150),
    sweep_interval: Duration::from_millis(25),
}
```

- [ ] **Step 2: Run tests and confirm failure**

```bash
cd foster-platform
cargo test -p foster-server --test agent_gateway -- --nocapture
```

Expected: FAIL because app/gateway modules are undefined.

- [ ] **Step 3: Implement Server configuration**

`config.rs`:

```rust
use std::{net::SocketAddr, time::Duration};

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub agent_gateway: AgentGatewayConfig,
}

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

Environment parsing must require:

- `FOSTER_BIND_ADDR`
- `FOSTER_DATABASE_URL`
- `FOSTER_AGENT_TOKEN`

Return a descriptive startup error when any required variable is missing.

- [ ] **Step 4: Implement the registry**

`registry.rs` public API:

```rust
#[derive(Debug, Clone)]
pub struct AgentPresence {
    pub agent_id: String,
    pub host_id: i64,
    pub connected_at: Instant,
    pub last_heartbeat_at: Instant,
}

#[derive(Clone, Default)]
pub struct AgentRegistry {
    inner: Arc<DashMap<i64, AgentPresence>>,
}

impl AgentRegistry {
    pub fn register(&self, presence: AgentPresence);
    pub fn heartbeat(&self, host_id: i64);
    pub fn remove(&self, host_id: i64);
    pub fn is_online(&self, host_id: i64) -> bool;
    pub fn connection_count(&self) -> usize;
    pub fn remove_stale(&self, timeout: Duration) -> Vec<i64>;
}
```

Registering a second connection for the same `host_id` replaces the logical presence rather than incrementing connection count.

- [ ] **Step 5: Implement Bearer auth and WebSocket handler**

Auth behavior:

- missing Authorization → 401
- wrong scheme → 401
- wrong token → 401
- correct `Bearer <token>` → permit upgrade

WebSocket behavior:

1. First valid message must be `AgentEvent::Hello`.
2. Validate `protocol_version`.
3. Register the host only after valid HELLO.
4. Accept subsequent `Heartbeat` only for the registered host id.
5. On socket close, remove that exact host presence if it still belongs to this logical connection.
6. Spawn a sweeper that calls `remove_stale(heartbeat_timeout)` every `sweep_interval`.

Use an internal connection generation UUID so an old socket closing cannot remove a newer replacement connection for the same host.

- [ ] **Step 6: Implement `/healthz` and application assembly**

`app.rs` exports:

```rust
#[derive(Clone)]
pub struct AppState {
    pub registry: AgentRegistry,
    pub gateway_config: AgentGatewayConfig,
}

pub fn build_app(state: AppState) -> Router;
```

Routes:

```text
GET /healthz
GET /agent/ws
```

- [ ] **Step 7: Add timeout test**

Extend `agent_gateway.rs`:

```rust
#[tokio::test]
async fn host_becomes_offline_after_heartbeat_timeout() {
    // start app with 150ms timeout
    // connect and send HELLO
    // assert online
    // send no heartbeat
    // wait until the test registry reports offline, bounded by 1 second
    // assert offline
}
```

This test uses a short injected timeout; production remains 45 seconds.

- [ ] **Step 8: Run all Server tests and lint**

```bash
cd foster-platform
cargo test -p foster-server -- --nocapture
cargo clippy -p foster-server --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add foster-platform/crates/server
git commit -m "feat: add agent websocket gateway"
```

---

### Task 7: Implement Agent emulator abstraction, HELLO/heartbeat loop, and reconnect behavior

**Files:**
- Create: `foster-platform/crates/agent/src/config.rs`
- Create: `foster-platform/crates/agent/src/runtime.rs`
- Create: `foster-platform/crates/agent/src/emulator/mod.rs`
- Create: `foster-platform/crates/agent/src/emulator/driver.rs`
- Create: `foster-platform/crates/agent/src/emulator/fake.rs`
- Create: `foster-platform/crates/agent/src/ws/mod.rs`
- Create: `foster-platform/crates/agent/src/ws/client.rs`
- Modify: `foster-platform/crates/agent/src/main.rs`
- Create: `foster-platform/crates/agent/tests/reconnect.rs`

**Interfaces:**
- Consumes: protocol from Task 5 and Server gateway behavior from Task 6.
- Produces:
  - `EmulatorDriver` abstraction
  - `AgentRuntime<D: EmulatorDriver>`
  - HELLO after each successful WebSocket connection
  - heartbeat every configured interval
  - reconnect backoff sequence capped at 30 seconds

- [ ] **Step 1: Write failing EmulatorDriver unit test**

`fake.rs` test:

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
```

- [ ] **Step 2: Define the driver trait and fake implementation**

`driver.rs`:

```rust
use async_trait::async_trait;
use foster_protocol::EmulatorDescriptor;

#[derive(Debug, thiserror::Error)]
pub enum EmulatorDriverError {
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

`FakeEmulatorDriver` returns its configured list, returns deterministic empty screenshot bytes, and accepts start/stop for known emulator codes.

- [ ] **Step 3: Run Agent unit tests**

```bash
cd foster-platform
cargo test -p foster-agent fake_driver_reports_configured_instances
```

Expected: PASS after implementation.

- [ ] **Step 4: Write failing reconnect integration test**

Create `tests/reconnect.rs`.

The test server must:

1. accept an authenticated WebSocket,
2. read one HELLO,
3. close the first connection,
4. accept the reconnect,
5. read a second HELLO,
6. assert both HELLO messages use the same `agent_id` and `host_id`.

Configure the Agent test runtime:

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

- [ ] **Step 5: Run reconnect test and confirm failure**

```bash
cd foster-platform
cargo test -p foster-agent --test reconnect -- --nocapture
```

Expected: FAIL because `AgentRuntime` and WebSocket client are undefined.

- [ ] **Step 6: Implement Agent configuration and WebSocket client**

`config.rs`:

```rust
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub server_ws_url: String,
    pub agent_token: String,
    pub agent_id: String,
    pub host_id: i64,
    pub heartbeat_interval: Duration,
    pub reconnect_delays: Vec<Duration>,
}

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

`WsClient::connect` must attach:

```text
Authorization: Bearer <agent_token>
```

and return a typed WebSocket stream.

- [ ] **Step 7: Implement AgentRuntime**

Public shape:

```rust
pub struct AgentRuntime<D: EmulatorDriver> {
    config: AgentConfig,
    driver: Arc<D>,
}

impl<D: EmulatorDriver> AgentRuntime<D> {
    pub fn new(config: AgentConfig, driver: D) -> Self;
    pub async fn run(self) -> Result<(), AgentRuntimeError>;
}
```

Per connection:

1. connect,
2. send HELLO,
3. immediately send `EmulatorSnapshot` from `driver.list_instances()`,
4. start heartbeat ticker,
5. read Server messages,
6. respond to `Ping` with `Pong`,
7. on disconnect, sleep according to configured reconnect sequence,
8. reset backoff index after a connection survives at least one heartbeat interval.

Phase 1 ignores `RefreshEmulators` only until it is received; when received it must send a fresh `EmulatorSnapshot`.

- [ ] **Step 8: Make `main.rs` runnable with FakeEmulatorDriver**

Read environment:

- `FOSTER_SERVER_WS_URL`
- `FOSTER_AGENT_TOKEN`
- `FOSTER_AGENT_ID`
- `FOSTER_HOST_ID`

For Phase 1, instantiate:

```rust
FakeEmulatorDriver::new(Vec::new())
```

Log clearly that this is the Phase 1 fake driver; do not claim real MuMu/LDPlayer support.

- [ ] **Step 9: Run reconnect test, all Agent tests, and lint**

```bash
cd foster-platform
cargo test -p foster-agent -- --nocapture
cargo clippy -p foster-agent --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add foster-platform/crates/agent
git commit -m "feat: add reconnecting foster agent runtime"
```

---

### Task 8: Persist HELLO/heartbeat state and emulator snapshots into MySQL

**Files:**
- Modify: `foster-platform/crates/server/src/control_plane/repository.rs`
- Modify: `foster-platform/crates/server/src/agent_gateway/handler.rs`
- Modify: `foster-platform/crates/server/src/app.rs`
- Modify: `foster-platform/crates/server/src/main.rs`
- Create: `foster-platform/crates/server/tests/agent_persistence.rs`

**Interfaces:**
- Consumes: valid HELLO, Heartbeat, and EmulatorSnapshot messages.
- Produces:
  - host ONLINE/OFFLINE persistence
  - host `agent_version` and `last_heartbeat_at`
  - emulator upsert by `emulator_code`
  - emulator `host_id`, `driver_type`, `adb_serial`, and heartbeat status updates

- [ ] **Step 1: Write failing persistence integration test**

Test flow:

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn hello_and_snapshot_persist_host_and_emulator(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    // seed host id 7 with OFFLINE status
    // start app using this pool
    // connect authenticated Agent
    // send HELLO for host id 7
    // send EmulatorSnapshot containing emu-01
    // query host: status == ONLINE, agent_version == 0.1.0
    // query emulator_instance: emulator_code == emu-01, host_id == 7
    // send Heartbeat
    // assert last_heartbeat_at is not null
    Ok(())
}
```

Add a second test:

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn hello_for_unknown_host_is_rejected(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    // do not seed host id
    // send HELLO for host 999
    // assert registry never reports host 999 online
    // assert no host row is auto-created
    Ok(())
}
```

This prevents a stolen Agent token from inventing arbitrary Host records.

- [ ] **Step 2: Run test and confirm failure**

```bash
cd foster-platform
export DATABASE_URL='mysql://root:foster@127.0.0.1:3307/mysql'
cargo test -p foster-server --test agent_persistence -- --nocapture
```

Expected: FAIL because the handler is not database-backed.

- [ ] **Step 3: Add repository operations**

Implement exact operations:

```rust
pub async fn host_exists(
    pool: &MySqlPool,
    host_id: i64,
) -> Result<bool, sqlx::Error>;

pub async fn mark_host_online(
    pool: &MySqlPool,
    host_id: i64,
    agent_version: &str,
) -> Result<(), sqlx::Error>;

pub async fn touch_host_heartbeat(
    pool: &MySqlPool,
    host_id: i64,
) -> Result<(), sqlx::Error>;

pub async fn mark_host_offline(
    pool: &MySqlPool,
    host_id: i64,
) -> Result<(), sqlx::Error>;

pub async fn upsert_emulator_snapshot(
    pool: &MySqlPool,
    host_id: i64,
    items: &[EmulatorDescriptor],
) -> Result<(), sqlx::Error>;
```

Upsert rule:

- key by globally unique `emulator_code`
- update `host_id`, `driver_type`, `adb_serial`, `last_heartbeat_at`
- do not overwrite `max_account_count`
- do not overwrite Server-controlled `current_job_id`

- [ ] **Step 4: Wire persistence into the gateway**

`AppState` gains:

```rust
pub pool: MySqlPool,
```

HELLO:

1. validate protocol,
2. verify `host_exists`,
3. persist ONLINE,
4. register connection.

Heartbeat:

1. update registry,
2. persist `last_heartbeat_at`.

EmulatorSnapshot:

1. verify snapshot host id equals authenticated connection host id,
2. upsert the emulator list.

Stale-host sweeper:

1. registry identifies stale host ids,
2. persist those hosts OFFLINE.

- [ ] **Step 5: Run Server integration tests**

```bash
cd foster-platform
export DATABASE_URL='mysql://root:foster@127.0.0.1:3307/mysql'
cargo test -p foster-server -- --nocapture
cargo clippy -p foster-server --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add foster-platform/crates/server
git commit -m "feat: persist agent and emulator presence"
```

---

### Task 9: Add Phase 1 local run documentation and full verification

**Files:**
- Create: `foster-platform/README.md`
- Modify: `foster-platform/.env.example`

**Interfaces:**
- Consumes: completed Phase 1 workspace.
- Produces: reproducible local bootstrap and verification instructions.

- [ ] **Step 1: Write the README with exact local commands**

Document this sequence:

```bash
cd foster-platform

docker compose -f docker-compose.dev.yml up -d mysql

export FOSTER_DATABASE_URL='mysql://root:foster@127.0.0.1:3307/foster'
export FOSTER_BIND_ADDR='127.0.0.1:18080'
export FOSTER_AGENT_TOKEN='dev-token'

sqlx migrate run --source migrations

cargo run -p foster-server
```

In a second terminal:

```bash
cd foster-platform

export FOSTER_SERVER_WS_URL='ws://127.0.0.1:18080/agent/ws'
export FOSTER_AGENT_TOKEN='dev-token'
export FOSTER_AGENT_ID='dev-agent-01'
export FOSTER_HOST_ID='1'

cargo run -p foster-agent
```

Before running Agent, seed one Host explicitly:

```sql
INSERT INTO host(host_code, hostname, status)
VALUES ('HOST-01', 'dev-windows-host', 'OFFLINE');
```

Document that the returned id must be used as `FOSTER_HOST_ID`.

Document Phase 1 limitations explicitly:

- fake emulator driver only
- no QR login
- no real foster execution
- no Scheduler
- no H5
- no platform resource allocation

- [ ] **Step 2: Run the complete automated verification suite**

Run:

```bash
cd foster-platform
cargo fmt --all --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 3: Run the MySQL-backed tests against the local MySQL service**

Run:

```bash
cd foster-platform
docker compose -f docker-compose.dev.yml up -d mysql
export DATABASE_URL='mysql://root:foster@127.0.0.1:3307/mysql'
cargo test -p foster-server --tests -- --nocapture
```

Expected: PASS.

- [ ] **Step 4: Perform the manual Server/Agent handshake smoke test**

With Server running and a seeded Host:

1. start Agent,
2. observe Server log for accepted HELLO,
3. observe Agent log for successful connection,
4. query:

```sql
SELECT id, host_code, status, agent_version, last_heartbeat_at
FROM host;

SELECT id, host_id, emulator_code, driver_type, status
FROM emulator_instance;
```

Expected:

- seeded Host is ONLINE,
- agent_version is populated,
- heartbeat timestamp advances,
- fake driver produces no emulator rows when configured empty.

Stop Agent and wait more than 45 seconds.

Expected:

- Host transitions to OFFLINE.

- [ ] **Step 5: Commit**

```bash
git add foster-platform/README.md foster-platform/.env.example
git commit -m "docs: add foster control plane local run guide"
```

---

## Phase 1 Completion Gate

Do not begin QR login work until all of these are true:

- `cargo fmt --all --check` passes.
- `cargo test --workspace --all-targets` passes.
- `cargo clippy --workspace --all-targets -- -D warnings` passes.
- MySQL schema tests prove one ACTIVE binding per game account.
- The concurrent last-slot allocation test proves no capacity oversell.
- Masked-account-only identity remains AMBIGUOUS.
- Strong identity conflicts produce MISMATCH.
- Invalid Agent tokens do not register connections.
- Unsupported protocol versions do not register connections.
- Agent reconnect test observes two HELLOs for one logical Agent identity.
- Host presence persists ONLINE on HELLO/heartbeat and becomes OFFLINE after timeout.
- Existing Python OAS tests remain unchanged and are not broken by the new `foster-platform/` directory.

After Phase 1, write separate implementation plans in this order:

1. **QR Login & Account Enrollment** — LoginSession, QR capture, SSE, user confirmation, identity enrollment, ACTIVE binding finalization.
2. **Foster Scheduler & Job State Machine** — subscriptions, quiet periods, manual pause, six-hour cadence, per-account/per-emulator serialization, retries.
3. **OAS Foster Bridge** — single-run execution contract, structured results, target-account verification, BASIC foster flow.
4. **Platform Resource Pool** — provider friends, resource cycles, atomic reservations, specified-provider foster flow.
5. **Customer H5 & Operations API** — secure share/control tokens, status/history, pause controls, quiet-period editor, re-login entrypoint.
