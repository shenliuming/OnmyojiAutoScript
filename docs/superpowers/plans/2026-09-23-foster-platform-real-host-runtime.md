# Foster Platform Real Host Runtime Implementation Plan

**Goal:** Replace fake host execution with a vendor-neutral Windows runtime based on static emulator inventory + ADB, and make QR enrollment truly two-stage.

## Scope

1. Static emulator inventory from `FOSTER_EMULATORS_JSON`.
2. Generic ADB driver:
   - list configured instances
   - optional vendor start/stop commands
   - wait for ADB
   - adb serial
   - PNG screenshot
   - optional game launch via package name
3. Two-stage LoginExecutor:
   - prepare_login -> screenshot Data URL immediately
   - wait_identity -> background polling
   - cancel
4. OAS `POST /login/detect`:
   - OCR current masked account from user center
   - OCR current server
   - OCR character list
   - accept exactly one distinct character
   - multiple/zero characters remain pending/ambiguous
5. Agent runtime:
   - login execution runs in background so heartbeat never stops
   - QR_READY emitted before identity wait
   - identity emitted only after strong detection
6. Production main:
   - no FakeEmulatorDriver
   - no missing LoginExecutor
   - GenericAdbEmulatorDriver + HttpOasLoginExecutor + HttpOasFosterExecutor
7. H5 already supports data:image QR payload; no frontend contract change.

## Emulator JSON

Example:

```json
[
  {
    "emulatorCode": "emu-01",
    "adbSerial": "127.0.0.1:16384",
    "oasConfigName": "oas-emu-01",
    "packageName": "com.netease.onmyoji",
    "startProgram": "C:\\path\\vendor-cli.exe",
    "startArgs": ["launch", "--index", "0"],
    "stopProgram": "C:\\path\\vendor-cli.exe",
    "stopArgs": ["quit", "--index", "0"]
  }
]
```

Vendor commands are optional. This keeps MuMu/LDPlayer choice open.

## Identity Detection

First enrollment requires:
- game UID, OR
- character + server.

Real host detector returns:
- masked account when observable;
- current server;
- exactly one OCR character from character list.

If zero or multiple distinct characters are detected, no identity event is emitted; polling continues until timeout.

## Safety

- Never store passwords.
- Never treat requested identity as detected identity.
- Never choose arbitrarily among multiple characters.
- Screenshot Data URL is ephemeral and only persists in the login_session until expiry.
- ADB and OAS endpoints are local host concerns.
- Start/stop command is configured as program + args, never shell text.
- Login background task must not block Agent heartbeat.
- Cancellation must stop identity polling.

## Tests

- static inventory parses and exposes descriptors;
- unknown emulator rejected;
- screenshot returns PNG bytes from fake adb command runner;
- LoginExecutor prepare returns data:image/png;base64;
- identity polling accepts unique character + server;
- ambiguous characters do not emit identity;
- cancel stops polling;
- heartbeat continues during login identity wait;
- main/config compile with real runtime.

## Final Gate

One final CI trigger:
- cargo fmt --all --check
- cargo check --workspace
- cargo test --workspace --all-targets
- cargo clippy --workspace --all-targets -- -D warnings
- Python compileall for login/foster bridge and touched OAS files.
