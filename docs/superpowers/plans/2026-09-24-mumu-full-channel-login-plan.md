# MuMu 全渠道扫码登录编排 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Foster Agent 自动选择空闲 MuMu 实例，设置 1280×720，通过 MuMu 应用市场安装并确认全渠道《阴阳师》，启动后向登录 H5 提供二维码。

**Architecture:** 在 Agent 侧增加 MuMu CLI 控制器、实例选择锁和全渠道包准备器；现有通用 ADB 驱动继续负责启动包、等待设备和截图。Server 登录状态机与 H5 保持不变，只接收 Agent 产生的 `QR_READY`、身份识别和失败事件。

**Tech Stack:** Rust 1.98、Tokio、Serde、现有 Foster protocol/runtime；Windows MuMu `mumu-cli.exe`、ADB、OAS FastAPI；Rust 单元测试和 Agent 集成测试。

**Spec:** `docs/superpowers/specs/2026-09-24-mumu-full-channel-login-design.md`

## Global Constraints

- 只允许启动 `com.netease.onmyoji.wyzymnqsd_cps`。
- 普通包 `com.netease.onmyoji` 在登录准备开始时从所有 MuMu 实例卸载。
- 分辨率固定为 1280×720。
- MuMu 应用市场包名为 `com.mumu.store`，主 Activity 为 `com.mumu.store/.MainActivity`。
- 应用市场没有明确选择“全渠道”或安装后包名不匹配时，必须失败，不得按默认位置盲点或启动普通包。
- Agent token、登录控制令牌和二维码原始数据不得写入普通日志。

## Review Focus

- 两个并发登录请求不能选择同一个 MuMu 实例；由 `InstanceLease` 测试覆盖。
- MuMu 实例已启动但 ADB 尚未在线时必须等待后再继续；由启动等待测试覆盖。
- 应用市场安装的是普通包或没有“全渠道”选项时必须硬失败；由包校验和市场选择测试覆盖。
- MuMu CLI 返回坏 JSON、非零退出码或超时时必须返回可读错误；由命令适配器测试覆盖。
- 登录准备失败/取消/Agent 重启时必须释放实例租约；由 runtime 生命周期测试覆盖。

---

### Task 1: 定义 MuMu 配置、命令结果和协议错误

**Files:**
- Create: `foster-platform/crates/agent/src/mumu/mod.rs`
- Create: `foster-platform/crates/agent/src/mumu/cli.rs`
- Modify: `foster-platform/crates/agent/src/lib.rs`
- Modify: `foster-platform/crates/agent/src/main.rs`
- Test: `foster-platform/crates/agent/src/mumu/cli.rs` unit tests

**Interfaces:**
- Produces `MumuConfig { cli_path, app_market_package, app_market_activity, normal_package, full_channel_package, resolution_width, resolution_height, command_timeout }`.
- Produces `MumuCli::info_all() -> Result<Vec<MumuInstanceInfo>, MumuError>` and `MumuCli::run(args: &[&str]) -> Result<CommandOutput, MumuError>`.
- `MumuInstanceInfo` contains `index`, `name`, `adb_host_ip`, `adb_port`, `is_android_started`, `is_process_started`, and `player_state`.

- [x] **Step 1: Write failing parser tests** for the observed `mumu-cli info --vmindex all` JSON, malformed JSON, non-zero exit, and timeout.
- [x] **Step 2: Run the focused Rust tests** with `cargo test -p foster-agent mumu::cli` and verify the missing parser/adapter fails.
- [x] **Step 3: Implement the typed CLI adapter** using `tokio::process::Command`, argument vectors, timeout, JSON parsing, and redacted error messages.
- [x] **Step 4: Add environment parsing** for `FOSTER_MUMU_CLI_PATH`, `FOSTER_MUMU_APP_MARKET_PACKAGE`, `FOSTER_MUMU_APP_MARKET_ACTIVITY`, `FOSTER_FULL_CHANNEL_PACKAGE`, `FOSTER_NORMAL_PACKAGE`, `FOSTER_RESOLUTION_WIDTH`, `FOSTER_RESOLUTION_HEIGHT`, and timeout values with defaults from the spec.
- [x] **Step 5: Run the focused tests** and commit as `feat: add MuMu CLI configuration adapter`.

### Task 2: Implement idle-instance discovery and leases

**Files:**
- Create: `foster-platform/crates/agent/src/mumu/lease.rs`
- Modify: `foster-platform/crates/agent/src/mumu/mod.rs`
- Modify: `foster-platform/crates/agent/src/runtime.rs`
- Test: `foster-platform/crates/agent/src/mumu/lease.rs` unit tests

**Interfaces:**
- Produces `InstanceLeaseManager::acquire(instances, active_command_instances) -> Result<InstanceLease, LeaseError>`.
- `InstanceLease` exposes the selected `MumuInstanceInfo`, computed ADB serial, and an async release guard.

- [x] **Step 1: Write failing lease tests** for first-idle selection, exclusion of active/ADB-offline instances, deterministic index ordering, and concurrent acquisition of one instance.
- [x] **Step 2: Run the lease tests** and verify they fail before the manager exists.
- [x] **Step 3: Implement the manager** with an async mutex keyed by MuMu index and explicit release on drop/cancel paths.
- [x] **Step 4: Integrate lease acquisition into login preparation** so the selected instance is held from preparation through QR expiry, confirmation, cancellation, or failure.
- [x] **Step 5: Run Agent tests** and commit as `feat: lease idle MuMu instances for login`.

### Task 3: Add resolution, package cleanup, and app-market installation

**Files:**
- Create: `foster-platform/crates/agent/src/mumu/preparer.rs`
- Create: `foster-platform/crates/agent/src/mumu/app_market.rs`
- Modify: `foster-platform/crates/agent/src/emulator/generic_adb.rs`
- Modify: `foster-platform/crates/agent/src/login/http.rs`
- Test: `foster-platform/crates/agent/src/mumu/preparer.rs` unit tests
- Test: `foster-platform/crates/agent/src/mumu/app_market.rs` unit tests

**Interfaces:**
- Produces `MumuLoginPreparer::prepare(instance) -> Result<PreparedInstance, PrepareError>`.
- Produces `AppMarketInstaller::install_full_channel(adb_serial) -> Result<(), InstallError>`.
- `PreparedInstance` contains the selected MuMu index, ADB serial, and OAS config name.

- [x] **Step 1: Write failing preparation tests** for resolution command generation, uninstalling `com.netease.onmyoji` on every discovered instance, rejecting a missing/incorrect full-channel package, and accepting `com.netease.onmyoji.wyzymnqsd_cps`.
- [x] **Step 2: Write failing app-market workflow tests** using a fake ADB runner for launching `com.mumu.store/.MainActivity`, finding the “阴阳师” search result, requiring the “全渠道” selection, confirming installation, and timing out safely.
- [x] **Step 3: Implement resolution and package hygiene** with `mumu-cli setting`, `adb shell pm uninstall`, and `adb shell pm list packages`; never call the game launch command before package validation passes.
- [x] **Step 4: Implement the app-market installer** behind a trait so tests use a fake UI tree; the real implementation uses ADB UI hierarchy/text queries and coordinate-independent selectors, not blind fixed taps.
- [x] **Step 5: Add package-install and market timeout/error messages** to the login executor and map them to the existing terminal login failure event.
- [x] **Step 6: Run focused tests** and commit as `feat: prepare MuMu full-channel package for login`.

### Task 4: Connect preparation to QR capture and runtime events

**Files:**
- Modify: `foster-platform/crates/agent/src/login/http.rs`
- Modify: `foster-platform/crates/agent/src/runtime.rs`
- Modify: `foster-platform/crates/agent/src/main.rs`
- Test: `foster-platform/crates/agent/tests/login_prepare.rs`

**Interfaces:**
- `HttpOasLoginExecutor::prepare` calls `MumuLoginPreparer`, launches only the validated full-channel package, waits for the login screen, then captures `adb exec-out screencap -p`.
- Existing `wait_identity` continues polling OAS `/login/detect` after QR scan.

- [x] **Step 1: Write a failing integration test** with fake MuMu/ADB/OAS adapters proving the order: lease → resolution → uninstall normal package → app-market full-channel install → launch full-channel package → screenshot.
- [x] **Step 2: Add failure tests** proving that a failed market install and wrong package stop before `monkey` launch and release the lease.
- [x] **Step 3: Implement the executor wiring** while preserving the existing `QR_READY` payload shape and login H5 contract.
- [x] **Step 4: Verify cancellation and timeout** release the lease and send the existing failure event without leaking screenshot data to logs.
- [x] **Step 5: Run Agent tests** and commit as `feat: wire MuMu preparation into QR login flow`.

### Task 5: Update deployment configuration and operator diagnostics

**Files:**
- Modify: `foster-platform/deploy/windows/agent.env.example`
- Modify: `foster-platform/deploy/windows/Test-FosterHost.ps1`
- Modify: `foster-platform/deploy/README.md`
- Create: `foster-platform/deploy/windows/Test-MumuFullChannel.ps1`

**Interfaces:**
- Example config documents the MuMu CLI path, `com.mumu.store/.MainActivity`, full-channel and normal package IDs, 1280×720 resolution, and timeouts.
- `Test-MumuFullChannel.ps1` performs read-only checks for CLI availability, instance discovery, package states, ADB reachability, and OAS reachability.

- [x] **Step 1: Add a failing documentation/config validation check** that required example keys and package IDs are present.
- [x] **Step 2: Update the example environment** with Windows paths matching `C:\Program Files\Netease\MuMu\nx_main\mumu-cli.exe` and the agreed defaults.
- [x] **Step 3: Implement the read-only preflight script** using `mumu-cli info`, ADB package queries, and OAS `/openapi.json`; it must not uninstall or install anything.
- [x] **Step 4: Document the destructive normal-package cleanup and the exact QR flow** in the deployment README.
- [x] **Step 5: Run PowerShell preflight and commit as `docs: document MuMu full-channel deployment`.

### Task 6: Verify on the real MuMu installation

**Files:**
- No product source changes unless a verification defect is found.
- Test outputs: `foster-platform/target/` and local runtime logs remain untracked/ignored.

- [x] **Step 1: Run the focused Rust unit and integration tests** for Agent and protocol crates.
- [x] **Step 2: Run `Test-MumuFullChannel.ps1`** and record the detected MuMu instances and package states.
- [ ] **Step 3: Execute one real login preparation** with the selected idle instance; verify resolution 1280×720, ordinary package removal, full-channel package presence, game launch, and `QR_READY`.
- [ ] **Step 4: Open the generated login H5, scan the QR code, and verify OAS `/login/detect` returns the account identity.
- [ ] **Step 5: Verify the service page and confirm no Agent token, control token, or QR base64 appears in logs.
- [x] **Step 6: Run the full workspace test command and report any Windows Application Control limitation separately from code failures.

