# Foster Platform Production Bootstrap Implementation Plan

**Goal:** Make the current MVP deployable for the first real Windows host without manual SQL or ad-hoc startup steps.

## Scope

1. Fix production environment templates.
2. Add database readiness endpoint.
3. Add authenticated Host bootstrap/admin API.
4. Add emulator inventory/capacity admin API so max account count never requires manual SQL.
5. Add Server production Docker image + compose stack.
6. Add Windows Agent env loader/start script + interactive-user scheduled-task installer.
7. Add first-host deployment and smoke-test documentation.
8. One final CI gate only.

## Task 1 — Environment templates

Server template must use the names actually consumed by `ServerConfig`:

- DATABASE_URL
- FOSTER_AGENT_TOKEN
- FOSTER_ADMIN_TOKEN
- FOSTER_BIND_ADDR
- FOSTER_SCHEDULER_INTERVAL_SECONDS
- FOSTER_RESOURCE_MIN_REMAINING_MINUTES
- FOSTER_RESOURCE_RETRY_SECONDS
- RUST_LOG

Agent template keeps:

- FOSTER_SERVER_WS_URL
- FOSTER_AGENT_TOKEN
- FOSTER_AGENT_ID
- FOSTER_HOST_ID
- FOSTER_OAS_BASE_URL
- FOSTER_ADB_PATH
- FOSTER_EMULATORS_JSON
- login/OAS timeouts

## Task 2 — Readiness

Add:

- GET /healthz: process liveness only.
- GET /readyz: SELECT 1 against MySQL.
  - 200 {"status":"ready"}
  - 503 {"status":"not_ready"}

No secrets in response.

## Task 3 — Host bootstrap API

Add authenticated admin API:

- POST /admin/hosts
- GET /admin/hosts

POST input:
- hostCode
- hostname
- optional status, default OFFLINE

POST is idempotent by hostCode and returns:
- id
- hostCode
- hostname
- status
- agentVersion
- lastHeartbeatAt

Rules:
- requires FOSTER_ADMIN_TOKEN
- no automatic Agent HELLO host creation
- hostCode and hostname required
- status accepts OFFLINE / MAINTENANCE; ONLINE is runtime-owned and cannot be forced through bootstrap
- updating an existing host must not overwrite agent_version or last_heartbeat_at

GET lists host status plus emulator counts:
- totalEmulators
- onlineEmulators
- configuredCapacity
- boundAccounts



### Emulator operations

- GET /admin/hosts/{host_id}/emulators
- PUT /admin/emulators/{emulator_id}/capacity

Capacity:
- integer 1..100;
- persists across Agent snapshots;
- removes the need for direct SQL when one emulator has a different account-slot limit.

## Task 4 — Server Docker deployment

Create:
- foster-platform/Dockerfile.server
- foster-platform/docker-compose.prod.yml
- foster-platform/server.env.example

Properties:
- multi-stage Rust build
- MySQL 8
- Server waits naturally through DB connection retries at container restart policy level
- Server auto-runs sqlx migrations
- healthcheck uses /readyz
- database is not exposed publicly by default
- persistent MySQL volume

## Task 5 — Windows Agent bootstrap

Create:
- foster-platform/deploy/windows/Start-FosterAgent.ps1
- foster-platform/deploy/windows/Install-FosterAgentTask.ps1
- foster-platform/deploy/windows/agent.env.example

Start script:
- read KEY=VALUE file without executing arbitrary shell text
- validate required variables
- resolve foster-agent.exe relative to script unless overridden
- run foreground and propagate exit code

Install script:
- create Windows Scheduled Task running at startup
- run in the configured interactive Windows user session
- trigger at user logon so GUI emulator vendor hooks stay in the desktop session
- working directory is deployment folder
- overwrite existing FosterAgent task idempotently
- do not default to SYSTEM/Session 0

## Task 6 — Deployment guide and smoke checklist

Create:
- foster-platform/deploy/README.md

Flow:
1. start MySQL + Server;
2. call /readyz;
3. bootstrap host via /admin/hosts;
4. place Agent/OAS on Windows host;
5. fill agent env with returned hostId;
6. start OAS;
7. start Agent;
8. verify /admin/hosts shows ONLINE and emulator inventory;
9. onboard one BASIC customer;
10. scan QR and confirm;
11. verify service H5;
12. verify one foster job reaches SUCCESS/RETRY.

## Tests

Rust:
- /readyz returns 200 with DB.
- host admin auth required.
- host bootstrap idempotent.
- host bootstrap cannot force ONLINE.
- existing runtime fields survive bootstrap update.
- host list returns emulator/capacity/binding aggregates.

Final CI:
- cargo fmt --all --check
- cargo check --workspace
- cargo test --workspace --all-targets
- cargo clippy --workspace --all-targets -- -D warnings
- existing Python compileall/provider tests
