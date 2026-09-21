# Foster Platform Data Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first independently testable foundation for the foster platform: an isolated Rust workspace, deterministic account-identity verification, MySQL control-plane schema, and transactional emulator-capacity allocation.

**Architecture:** Add `foster-platform/` beside the existing Python OAS code so the fork remains easy to sync with upstream. This plan establishes domain rules and persistence invariants before any network protocol or game execution code is added. The next plan will build Server ↔ Agent WebSocket communication on top of these stable types and tables.

**Tech Stack:** Rust stable, Tokio, SQLx 0.8 + MySQL 8.0, Serde, Chrono, Thiserror, Docker Compose.

**Spec:** `docs/superpowers/specs/2026-09-21-onmyoji-foster-platform-design.md`

## Global Constraints

- Work only on branch `feat/foster-platform`; keep `master` suitable for upstream fork synchronization.
- Do not modify existing Python OAS behavior in this plan.
- One emulator may hold multiple game accounts and has configurable `max_account_count`.
- One `GameAccount` may have at most one ACTIVE emulator binding at any instant.
- Capacity counts `PENDING`, `ACTIVE`, and `MIGRATING` bindings.
- A normal allocation must refuse an account that already has `PENDING`, `ACTIVE`, or `MIGRATING` binding state.
- Masked account text or OCR alias alone is never sufficient to verify an account.
- `GAME_UID` is the strongest identity factor; a detected UID conflict is a mismatch.
- Matching `CHARACTER_NAME + SERVER_NAME` is sufficient to verify.
- Matching masked/OCR account plus either character or server is sufficient to verify.
- Default business timezone remains `Asia/Shanghai`, though this plan does not yet implement scheduling.
- No WebSocket, QR login, scheduler, resource pool, H5, emulator-vendor driver, or OAS execution is implemented in this plan.
- Every behavior change follows TDD: failing test, confirm failure, minimal implementation, confirm pass, commit.

## Review Focus

- Two different people may expose the same masked account text; masked-only evidence must remain `Ambiguous`. Task 2 pins this with `masked_account_alone_is_ambiguous`.
- A stale or wrong UID must override a matching masked account and produce `Mismatch`. Task 2 pins this with `uid_conflict_is_mismatch_even_when_mask_matches`.
- Two concurrent customers racing for the final emulator slot must produce one success and one `NoCapacity`. Task 4 pins this with a concurrency integration test.
- Re-running ordinary allocation for the same account must not reserve another emulator. Task 4 pins this with `same_account_cannot_receive_second_reserved_binding`.
- Direct database writes must still reject two ACTIVE bindings for one account and two occupied bindings for one emulator slot. Task 3 pins both invariants with MySQL integration tests.

---

## File Structure

```text
foster-platform/
├── Cargo.toml
├── rust-toolchain.toml
├── .env.example
├── docker-compose.dev.yml
├── migrations/
│   └── 0001_control_plane.sql
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
    │   └── src/lib.rs
    ├── server/
    │   ├── Cargo.toml
    │   ├── src/
    │   │   ├── main.rs
    │   │   └── control_plane/
    │   │       ├── mod.rs
    │   │       ├── allocator.rs
    │   │       └── repository.rs
    │   └── tests/
    │       ├── schema_invariants.rs
    │       └── binding_allocator.rs
    └── agent/
        ├── Cargo.toml
        └── src/main.rs
```

`protocol` and `agent` are created as empty compile-safe workspace packages here so later plans do not need to restructure the workspace. They gain real behavior only in the WebSocket plan.

---

### Task 1: Create the isolated Rust workspace

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
- Produces: workspace packages `foster-domain`, `foster-protocol`, `foster-server`, `foster-agent`.

- [ ] **Step 1: Confirm the workspace is absent**

Run:

```bash
test ! -f foster-platform/Cargo.toml
```

Expected: exit code 0.

- [ ] **Step 2: Create the root workspace manifest**

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
anyhow = "1"
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
sqlx = { version = "0.8", features = [
    "runtime-tokio-rustls",
    "mysql",
    "chrono",
    "macros",
    "migrate",
] }
thiserror = "2"
tokio = { version = "1", features = [
    "macros",
    "rt-multi-thread",
    "sync",
    "time",
    "test-util",
] }
```

Create `foster-platform/rust-toolchain.toml`:

```toml
[toolchain]
channel = "stable"
profile = "minimal"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 3: Create the four package manifests**

Create `crates/domain/Cargo.toml`:

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

Create `crates/protocol/Cargo.toml`:

```toml
[package]
name = "foster-protocol"
version = "0.1.0"
edition.workspace = true

[dependencies]
foster-domain = { path = "../domain" }
serde.workspace = true
```

Create `crates/server/Cargo.toml`:

```toml
[package]
name = "foster-server"
version = "0.1.0"
edition.workspace = true

[dependencies]
foster-domain = { path = "../domain" }
sqlx.workspace = true
thiserror.workspace = true
tokio.workspace = true

[dev-dependencies]
anyhow.workspace = true
```

Create `crates/agent/Cargo.toml`:

```toml
[package]
name = "foster-agent"
version = "0.1.0"
edition.workspace = true

[dependencies]
foster-domain = { path = "../domain" }
foster-protocol = { path = "../protocol" }
```

- [ ] **Step 4: Create compile-safe entrypoints**

`crates/domain/src/lib.rs`:

```rust
pub const DOMAIN_VERSION: &str = env!("CARGO_PKG_VERSION");
```

`crates/protocol/src/lib.rs`:

```rust
pub const PROTOCOL_VERSION: u16 = 1;
```

`crates/server/src/main.rs`:

```rust
fn main() {
    println!("foster-server");
}
```

`crates/agent/src/main.rs`:

```rust
fn main() {
    println!("foster-agent");
}
```

- [ ] **Step 5: Verify the workspace**

Run:

```bash
cd foster-platform
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add foster-platform
git commit -m "build: add foster platform rust workspace"
```

---

### Task 2: Implement domain enums and account identity verification

**Files:**
- Create: `foster-platform/crates/domain/src/host.rs`
- Create: `foster-platform/crates/domain/src/emulator.rs`
- Create: `foster-platform/crates/domain/src/account.rs`
- Create: `foster-platform/crates/domain/src/binding.rs`
- Create: `foster-platform/crates/domain/src/identity.rs`
- Modify: `foster-platform/crates/domain/src/lib.rs`

**Interfaces:**
- Consumes: none.
- Produces:
  - `HostStatus`
  - `EmulatorStatus`
  - `LoginStatus`
  - `VerifyStatus`
  - `BindingStatus`
  - `IdentityType`
  - `AccountIdentity`
  - `DetectedIdentity`
  - `IdentityDecision`
  - `normalize_identity(kind, value)`
  - `verify_identity(stored, detected)`

- [ ] **Step 1: Create the enum modules**

Create `host.rs`:

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

Create `emulator.rs`:

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

Create `account.rs`:

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

Create `binding.rs`:

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

- [ ] **Step 2: Write the failing identity tests before the implementation**

Create `identity.rs` with the type declarations and tests below, but leave `verify_identity` absent so the test compile fails:

```rust
use std::collections::HashSet;

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
            ..Default::default()
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
            ..Default::default()
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
            character_name: Some("柳某某".into()),
            server_name: Some("春之樱".into()),
            ..Default::default()
        };

        assert!(matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Verified { .. }
        ));
    }

    #[test]
    fn uid_match_verifies() {
        let stored = vec![identity(IdentityType::GameUid, "10001")];
        let detected = DetectedIdentity {
            game_uid: Some("10001".into()),
            ..Default::default()
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
            game_uid: Some("99999".into()),
            ..Default::default()
        };

        assert!(matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Mismatch { .. }
        ));
    }

    #[test]
    fn character_conflict_is_mismatch() {
        let stored = vec![
            identity(IdentityType::MaskedAccount, "138****5678"),
            identity(IdentityType::CharacterName, "角色A"),
        ];
        let detected = DetectedIdentity {
            masked_account: Some("138****5678".into()),
            character_name: Some("角色B".into()),
            ..Default::default()
        };

        assert!(matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Mismatch { .. }
        ));
    }

    #[test]
    fn ocr_alias_plus_server_verifies() {
        let stored = vec![
            identity(IdentityType::OcrAlias, "138****S678"),
            identity(IdentityType::ServerName, "春之樱"),
        ];
        let detected = DetectedIdentity {
            server_name: Some("春之樱".into()),
            ocr_aliases: vec!["138****S678".into()],
            ..Default::default()
        };

        assert!(matches!(
            verify_identity(&stored, &detected),
            IdentityDecision::Verified { .. }
        ));
    }
}
```

- [ ] **Step 3: Run the identity tests and confirm compile failure**

Run:

```bash
cd foster-platform
cargo test -p foster-domain identity::tests
```

Expected: FAIL with an unresolved `verify_identity` function.

- [ ] **Step 4: Add the identity implementation**

Insert these helpers and `verify_identity` above the test module:

```rust
fn stored_values(
    stored: &[AccountIdentity],
    kind: IdentityType,
) -> HashSet<&str> {
    stored
        .iter()
        .filter(|identity| identity.kind == kind)
        .map(|identity| identity.normalized_value.as_str())
        .collect()
}

fn optional_match(
    stored: &[AccountIdentity],
    kind: IdentityType,
    detected: Option<&str>,
) -> bool {
    let Some(detected) = detected else {
        return false;
    };
    let expected = stored_values(stored, kind);
    if expected.is_empty() {
        return false;
    }
    let normalized = normalize_identity(kind, detected);
    expected.contains(normalized.as_str())
}

fn optional_conflict(
    stored: &[AccountIdentity],
    kind: IdentityType,
    detected: Option<&str>,
) -> bool {
    let Some(detected) = detected else {
        return false;
    };
    let expected = stored_values(stored, kind);
    if expected.is_empty() {
        return false;
    }
    let normalized = normalize_identity(kind, detected);
    !expected.contains(normalized.as_str())
}

fn account_signal_matches(
    stored: &[AccountIdentity],
    detected: &DetectedIdentity,
) -> bool {
    let expected: HashSet<&str> = stored
        .iter()
        .filter(|identity| {
            matches!(
                identity.kind,
                IdentityType::MaskedAccount | IdentityType::OcrAlias
            )
        })
        .map(|identity| identity.normalized_value.as_str())
        .collect();

    if expected.is_empty() {
        return false;
    }

    let mut candidates = Vec::new();
    if let Some(masked) = detected.masked_account.as_deref() {
        candidates.push(normalize_identity(IdentityType::MaskedAccount, masked));
    }
    candidates.extend(
        detected
            .ocr_aliases
            .iter()
            .map(|alias| normalize_identity(IdentityType::OcrAlias, alias)),
    );

    candidates
        .iter()
        .any(|candidate| expected.contains(candidate.as_str()))
}

pub fn verify_identity(
    stored: &[AccountIdentity],
    detected: &DetectedIdentity,
) -> IdentityDecision {
    let mut conflicting = Vec::new();

    if optional_conflict(
        stored,
        IdentityType::GameUid,
        detected.game_uid.as_deref(),
    ) {
        conflicting.push(IdentityType::GameUid);
    }
    if optional_conflict(
        stored,
        IdentityType::CharacterName,
        detected.character_name.as_deref(),
    ) {
        conflicting.push(IdentityType::CharacterName);
    }
    if optional_conflict(
        stored,
        IdentityType::ServerName,
        detected.server_name.as_deref(),
    ) {
        conflicting.push(IdentityType::ServerName);
    }

    if !conflicting.is_empty() {
        return IdentityDecision::Mismatch { conflicting };
    }

    let uid_match = optional_match(
        stored,
        IdentityType::GameUid,
        detected.game_uid.as_deref(),
    );
    let character_match = optional_match(
        stored,
        IdentityType::CharacterName,
        detected.character_name.as_deref(),
    );
    let server_match = optional_match(
        stored,
        IdentityType::ServerName,
        detected.server_name.as_deref(),
    );
    let account_match = account_signal_matches(stored, detected);

    let mut matched = Vec::new();
    if uid_match {
        matched.push(IdentityType::GameUid);
    }
    if character_match {
        matched.push(IdentityType::CharacterName);
    }
    if server_match {
        matched.push(IdentityType::ServerName);
    }
    if account_match {
        matched.push(IdentityType::MaskedAccount);
    }

    if uid_match
        || (character_match && server_match)
        || (account_match && (character_match || server_match))
    {
        IdentityDecision::Verified { matched }
    } else {
        IdentityDecision::Ambiguous
    }
}
```

- [ ] **Step 5: Export the domain modules**

Replace `domain/src/lib.rs` with:

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

- [ ] **Step 6: Verify Task 2**

Run:

```bash
cd foster-platform
cargo test -p foster-domain
cargo fmt --all --check
cargo clippy -p foster-domain --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add foster-platform/crates/domain
git commit -m "feat: add account identity verification rules"
```

---

### Task 3: Add MySQL schema and database-enforced binding invariants

**Files:**
- Create: `foster-platform/migrations/0001_control_plane.sql`
- Create: `foster-platform/docker-compose.dev.yml`
- Create: `foster-platform/.env.example`
- Create: `foster-platform/crates/server/tests/schema_invariants.rs`

**Interfaces:**
- Consumes: binding status names from Task 2.
- Produces tables:
  - `host`
  - `emulator_instance`
  - `game_account`
  - `game_account_identity`
  - `emulator_account_binding`

- [ ] **Step 1: Create local MySQL service and environment example**

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
      test: ["CMD", "mysqladmin", "ping", "-h127.0.0.1", "-uroot", "-pfoster"]
      interval: 3s
      timeout: 3s
      retries: 20
```

Create `.env.example`:

```dotenv
FOSTER_DATABASE_URL=mysql://root:foster@127.0.0.1:3307/foster
```

- [ ] **Step 2: Write failing schema tests before creating the migration**

Create `crates/server/tests/schema_invariants.rs`:

```rust
use sqlx::MySqlPool;

async fn insert_host(pool: &MySqlPool, code: &str) -> sqlx::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES (?, ?, 'OFFLINE')",
    )
    .bind(code)
    .bind(code)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn insert_emulator(
    pool: &MySqlPool,
    host_id: i64,
    code: &str,
    max_account_count: i32,
) -> sqlx::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, ?, 'UNKNOWN', ?, 'IDLE')",
    )
    .bind(host_id)
    .bind(code)
    .bind(max_account_count)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn insert_account(pool: &MySqlPool, customer_id: i64) -> sqlx::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status
         )
         VALUES (?, 'PENDING', 'PENDING')",
    )
    .bind(customer_id)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

async fn insert_binding(
    pool: &MySqlPool,
    emulator_id: i64,
    game_account_id: i64,
    slot_no: i32,
    status: &str,
) -> sqlx::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO emulator_account_binding(
            emulator_id, game_account_id, slot_no, status
         )
         VALUES (?, ?, ?, ?)",
    )
    .bind(emulator_id)
    .bind(game_account_id)
    .bind(slot_no)
    .bind(status)
    .execute(pool)
    .await?;

    Ok(result.last_insert_id() as i64)
}

#[sqlx::test(migrations = "../../migrations")]
async fn database_rejects_two_active_bindings_for_same_account(
    pool: MySqlPool,
) -> sqlx::Result<()> {
    let host_id = insert_host(&pool, "host-a").await?;
    let emu_a = insert_emulator(&pool, host_id, "emu-a", 5).await?;
    let emu_b = insert_emulator(&pool, host_id, "emu-b", 5).await?;
    let account_id = insert_account(&pool, 1001).await?;

    insert_binding(&pool, emu_a, account_id, 1, "ACTIVE").await?;

    let second =
        insert_binding(&pool, emu_b, account_id, 1, "ACTIVE").await;
    assert!(second.is_err());

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn database_rejects_two_occupied_bindings_for_same_slot(
    pool: MySqlPool,
) -> sqlx::Result<()> {
    let host_id = insert_host(&pool, "host-a").await?;
    let emulator_id = insert_emulator(&pool, host_id, "emu-a", 5).await?;
    let account_a = insert_account(&pool, 1001).await?;
    let account_b = insert_account(&pool, 1002).await?;

    insert_binding(&pool, emulator_id, account_a, 1, "PENDING").await?;

    let second =
        insert_binding(&pool, emulator_id, account_b, 1, "PENDING").await;
    assert!(second.is_err());

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn unbound_history_does_not_block_slot_reuse(
    pool: MySqlPool,
) -> sqlx::Result<()> {
    let host_id = insert_host(&pool, "host-a").await?;
    let emulator_id = insert_emulator(&pool, host_id, "emu-a", 5).await?;
    let account_a = insert_account(&pool, 1001).await?;
    let account_b = insert_account(&pool, 1002).await?;

    insert_binding(&pool, emulator_id, account_a, 1, "UNBOUND").await?;
    insert_binding(&pool, emulator_id, account_b, 1, "PENDING").await?;

    Ok(())
}
```

- [ ] **Step 3: Start MySQL and confirm the tests fail without the migration**

Run:

```bash
cd foster-platform
docker compose -f docker-compose.dev.yml up -d mysql
export DATABASE_URL='mysql://root:foster@127.0.0.1:3307/mysql'
cargo test -p foster-server --test schema_invariants -- --nocapture
```

Expected: FAIL because the migration file and tables do not exist.

- [ ] **Step 4: Create the migration**

Create `migrations/0001_control_plane.sql`:

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
            CASE
                WHEN status = 'ACTIVE' THEN game_account_id
                ELSE NULL
            END
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

- [ ] **Step 5: Re-run schema tests**

Run:

```bash
cd foster-platform
export DATABASE_URL='mysql://root:foster@127.0.0.1:3307/mysql'
cargo test -p foster-server --test schema_invariants -- --nocapture
```

Expected: all three tests PASS.

- [ ] **Step 6: Commit**

```bash
git add foster-platform/migrations foster-platform/docker-compose.dev.yml foster-platform/.env.example foster-platform/crates/server/tests/schema_invariants.rs
git commit -m "feat: add foster control plane schema"
```

---

### Task 4: Implement transactional emulator-capacity allocation

**Files:**
- Create: `foster-platform/crates/server/src/control_plane/mod.rs`
- Create: `foster-platform/crates/server/src/control_plane/repository.rs`
- Create: `foster-platform/crates/server/src/control_plane/allocator.rs`
- Modify: `foster-platform/crates/server/src/main.rs`
- Create: `foster-platform/crates/server/tests/binding_allocator.rs`

**Interfaces:**
- Consumes: MySQL schema from Task 3.
- Produces:
  - `BindingAllocator::new(MySqlPool)`
  - `BindingAllocator::allocate_pending(game_account_id)`
  - `AllocatedBinding { binding_id, emulator_id, slot_no }`
  - `AllocationError::{AccountNotFound, AlreadyBound, NoCapacity, Database}`

- [ ] **Step 1: Write the failing allocator tests**

Create `tests/binding_allocator.rs`:

```rust
use foster_server::control_plane::{
    AllocationError, BindingAllocator,
};
use sqlx::MySqlPool;

async fn seed_host(pool: &MySqlPool) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO host(host_code, hostname, status)
         VALUES ('host-a', 'host-a', 'ONLINE')",
    )
    .execute(pool)
    .await?;
    Ok(result.last_insert_id() as i64)
}

async fn seed_emulator(
    pool: &MySqlPool,
    host_id: i64,
    code: &str,
    capacity: i32,
) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO emulator_instance(
            host_id, emulator_code, driver_type,
            max_account_count, status
         )
         VALUES (?, ?, 'UNKNOWN', ?, 'IDLE')",
    )
    .bind(host_id)
    .bind(code)
    .bind(capacity)
    .execute(pool)
    .await?;
    Ok(result.last_insert_id() as i64)
}

async fn seed_account(
    pool: &MySqlPool,
    customer_id: i64,
) -> anyhow::Result<i64> {
    let result = sqlx::query(
        "INSERT INTO game_account(
            customer_id, login_status, verify_status
         )
         VALUES (?, 'PENDING', 'PENDING')",
    )
    .bind(customer_id)
    .execute(pool)
    .await?;
    Ok(result.last_insert_id() as i64)
}

#[sqlx::test(migrations = "../../migrations")]
async fn pending_binding_consumes_capacity(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let host_id = seed_host(&pool).await?;
    seed_emulator(&pool, host_id, "emu-a", 1).await?;
    let account_a = seed_account(&pool, 1001).await?;
    let account_b = seed_account(&pool, 1002).await?;

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
    let host_id = seed_host(&pool).await?;
    seed_emulator(&pool, host_id, "emu-a", 5).await?;
    seed_emulator(&pool, host_id, "emu-b", 5).await?;
    let account = seed_account(&pool, 1001).await?;

    let allocator = BindingAllocator::new(pool.clone());
    allocator.allocate_pending(account).await?;

    let second = allocator.allocate_pending(account).await;
    assert!(matches!(second, Err(AllocationError::AlreadyBound)));

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn allocator_uses_lowest_free_slot(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let host_id = seed_host(&pool).await?;
    let emulator_id = seed_emulator(&pool, host_id, "emu-a", 3).await?;
    let existing = seed_account(&pool, 1001).await?;
    let newcomer = seed_account(&pool, 1002).await?;

    sqlx::query(
        "INSERT INTO emulator_account_binding(
            emulator_id, game_account_id, slot_no, status
         )
         VALUES (?, ?, 1, 'ACTIVE')",
    )
    .bind(emulator_id)
    .bind(existing)
    .execute(&pool)
    .await?;

    let allocator = BindingAllocator::new(pool.clone());
    let allocated = allocator.allocate_pending(newcomer).await?;

    assert_eq!(allocated.emulator_id, emulator_id);
    assert_eq!(allocated.slot_no, 2);

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_allocations_do_not_oversell_last_slot(
    pool: MySqlPool,
) -> anyhow::Result<()> {
    let host_id = seed_host(&pool).await?;
    seed_emulator(&pool, host_id, "emu-a", 1).await?;
    let account_a = seed_account(&pool, 1001).await?;
    let account_b = seed_account(&pool, 1002).await?;

    let allocator_a = BindingAllocator::new(pool.clone());
    let allocator_b = BindingAllocator::new(pool.clone());

    let (left, right) = tokio::join!(
        allocator_a.allocate_pending(account_a),
        allocator_b.allocate_pending(account_b)
    );

    let success_count = [left.is_ok(), right.is_ok()]
        .into_iter()
        .filter(|value| *value)
        .count();

    assert_eq!(success_count, 1);

    let occupied: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM emulator_account_binding
         WHERE status IN ('PENDING', 'ACTIVE', 'MIGRATING')",
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(occupied, 1);

    Ok(())
}
```

- [ ] **Step 2: Expose a Server library module so integration tests can import allocator code**

Create `crates/server/src/lib.rs`:

```rust
pub mod control_plane;
```

Keep `main.rs` minimal:

```rust
fn main() {
    println!("foster-server");
}
```

Run:

```bash
cd foster-platform
export DATABASE_URL='mysql://root:foster@127.0.0.1:3307/mysql'
cargo test -p foster-server --test binding_allocator -- --nocapture
```

Expected: FAIL because `control_plane` implementation does not yet exist.

- [ ] **Step 3: Implement repository primitives**

Create `control_plane/repository.rs`:

```rust
use sqlx::{MySql, Transaction};

#[derive(Debug, sqlx::FromRow)]
pub struct GameAccountLockRow {
    pub id: i64,
    pub active_emulator_id: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct EmulatorCapacityRow {
    pub id: i64,
    pub max_account_count: i32,
}

pub async fn lock_game_account(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
) -> Result<Option<GameAccountLockRow>, sqlx::Error> {
    sqlx::query_as::<_, GameAccountLockRow>(
        "SELECT id, active_emulator_id
         FROM game_account
         WHERE id = ?
         FOR UPDATE",
    )
    .bind(game_account_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn has_reserved_binding(
    tx: &mut Transaction<'_, MySql>,
    game_account_id: i64,
) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM emulator_account_binding
         WHERE game_account_id = ?
           AND status IN ('PENDING', 'ACTIVE', 'MIGRATING')",
    )
    .bind(game_account_id)
    .fetch_one(&mut **tx)
    .await?;

    Ok(count > 0)
}

pub async fn candidate_emulator_ids(
    tx: &mut Transaction<'_, MySql>,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id
         FROM emulator_instance
         WHERE status = 'IDLE'
         ORDER BY id ASC",
    )
    .fetch_all(&mut **tx)
    .await
}

pub async fn lock_emulator(
    tx: &mut Transaction<'_, MySql>,
    emulator_id: i64,
) -> Result<Option<EmulatorCapacityRow>, sqlx::Error> {
    sqlx::query_as::<_, EmulatorCapacityRow>(
        "SELECT id, max_account_count
         FROM emulator_instance
         WHERE id = ?
           AND status = 'IDLE'
         FOR UPDATE SKIP LOCKED",
    )
    .bind(emulator_id)
    .fetch_optional(&mut **tx)
    .await
}

pub async fn occupied_slots(
    tx: &mut Transaction<'_, MySql>,
    emulator_id: i64,
) -> Result<Vec<i32>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT slot_no
         FROM emulator_account_binding
         WHERE emulator_id = ?
           AND status IN ('PENDING', 'ACTIVE', 'MIGRATING')
         ORDER BY slot_no ASC",
    )
    .bind(emulator_id)
    .fetch_all(&mut **tx)
    .await
}

pub async fn insert_pending_binding(
    tx: &mut Transaction<'_, MySql>,
    emulator_id: i64,
    game_account_id: i64,
    slot_no: i32,
) -> Result<i64, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO emulator_account_binding(
            emulator_id, game_account_id, slot_no, status
         )
         VALUES (?, ?, ?, 'PENDING')",
    )
    .bind(emulator_id)
    .bind(game_account_id)
    .bind(slot_no)
    .execute(&mut **tx)
    .await?;

    Ok(result.last_insert_id() as i64)
}
```

Create `control_plane/mod.rs`:

```rust
mod allocator;
mod repository;

pub use allocator::{
    AllocatedBinding, AllocationError, BindingAllocator,
};
```

- [ ] **Step 4: Implement the allocator transaction**

Create `control_plane/allocator.rs`:

```rust
use std::collections::HashSet;

use sqlx::MySqlPool;

use super::repository::{
    candidate_emulator_ids, has_reserved_binding, insert_pending_binding,
    lock_emulator, lock_game_account, occupied_slots,
};

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
        let mut tx = self.pool.begin().await?;

        let account = lock_game_account(&mut tx, game_account_id)
            .await?
            .ok_or(AllocationError::AccountNotFound)?;

        if account.active_emulator_id.is_some()
            || has_reserved_binding(&mut tx, game_account_id).await?
        {
            tx.rollback().await?;
            return Err(AllocationError::AlreadyBound);
        }

        let candidate_ids = candidate_emulator_ids(&mut tx).await?;

        for emulator_id in candidate_ids {
            let Some(emulator) =
                lock_emulator(&mut tx, emulator_id).await?
            else {
                continue;
            };

            let occupied = occupied_slots(&mut tx, emulator.id).await?;
            if occupied.len() >= emulator.max_account_count as usize {
                continue;
            }

            let occupied: HashSet<i32> = occupied.into_iter().collect();
            let Some(slot_no) = (1..=emulator.max_account_count)
                .find(|slot| !occupied.contains(slot))
            else {
                continue;
            };

            let binding_id = match insert_pending_binding(
                &mut tx,
                emulator.id,
                game_account_id,
                slot_no,
            )
            .await
            {
                Ok(id) => id,
                Err(error) if is_duplicate_key(&error) => {
                    tx.rollback().await?;
                    return Err(AllocationError::NoCapacity);
                }
                Err(error) => return Err(AllocationError::Database(error)),
            };

            tx.commit().await?;

            return Ok(AllocatedBinding {
                binding_id,
                emulator_id: emulator.id,
                slot_no,
            });
        }

        tx.rollback().await?;
        Err(AllocationError::NoCapacity)
    }
}

fn is_duplicate_key(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(database_error) => {
            database_error.code().as_deref() == Some("1062")
        }
        _ => false,
    }
}
```

- [ ] **Step 5: Run allocator tests**

Run:

```bash
cd foster-platform
export DATABASE_URL='mysql://root:foster@127.0.0.1:3307/mysql'
cargo test -p foster-server --test binding_allocator -- --nocapture
```

Expected: all four tests PASS.

- [ ] **Step 6: Run the full Phase 1 verification**

Run:

```bash
cd foster-platform
cargo fmt --all --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

With MySQL available and `DATABASE_URL` set, expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add foster-platform/crates/server
git commit -m "feat: allocate emulator account capacity transactionally"
```

---

## Phase 1 Completion Gate

Do not write the WebSocket plan until all of these are true:

- Rust workspace compiles without touching existing Python OAS code.
- Identity unit tests pass.
- Masked-only identity returns `Ambiguous`.
- UID conflict returns `Mismatch`.
- Character + server can verify an account.
- MySQL rejects two ACTIVE bindings for one game account.
- MySQL rejects two occupied bindings for one emulator slot.
- Historical `UNBOUND` rows do not block slot reuse.
- A `PENDING` binding consumes emulator capacity.
- Same account cannot receive a second ordinary reservation.
- Two concurrent allocations cannot oversell a one-slot emulator.
- `cargo fmt --all --check` passes.
- `cargo test --workspace --all-targets` passes.
- `cargo clippy --workspace --all-targets -- -D warnings` passes.

## Subsequent Independent Plans

After this plan is implemented and reviewed, create separate plans in this order:

1. **Agent WebSocket Control Plane** — versioned protocol, Agent auth, HELLO/heartbeat, host/emulator presence, reconnect and state reconciliation foundation.
2. **QR Login & Account Enrollment** — LoginSession, QR capture, SSE, user confirmation, identity enrollment, ACTIVE binding finalization.
3. **Foster Scheduler & Job State Machine** — subscriptions, quiet periods, manual pause, six-hour cadence, per-account/per-emulator serialization and retry policy.
4. **OAS Foster Bridge** — structured single-run OAS execution, account verification, BASIC user-friend foster.
5. **Platform Resource Pool** — provider friend binding, resource cycles, atomic reservation, specified-provider foster.
6. **Customer H5 & Operations API** — share/control tokens, status/history, quiet-period editor, pause/re-login controls.
