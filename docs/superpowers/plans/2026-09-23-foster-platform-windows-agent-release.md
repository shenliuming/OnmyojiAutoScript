# Foster Platform Windows Agent Release / Smoke Implementation Plan

**Goal:** Make first-host rollout reproducible without installing Rust on every Windows machine.

## Scope

1. Add a manual-only GitHub Actions workflow that builds the Windows Agent package.
2. Package:
   - foster-agent.exe
   - Start-FosterAgent.ps1
   - Install-FosterAgentTask.ps1
   - Test-FosterHost.ps1
   - agent.env.example
   - build metadata + SHA-256 checksum
3. Add a Windows preflight/smoke script.
4. Update deployment docs with release download/use flow.
5. Existing Phase CI validates PowerShell syntax. Release workflow is never triggered by ordinary pushes.

## Release Workflow

File:
- .github/workflows/foster-agent-windows-package.yml

Trigger:
- workflow_dispatch only.

Inputs:
- package_label, optional human-readable label.

Build:
- windows-latest
- stable Rust
- cargo build --release -p foster-agent
- package deployment helpers
- compute SHA-256
- upload zip as Actions artifact

No GitHub Release is created automatically.
No secrets are embedded in the package.

## Smoke Script

Test-FosterHost.ps1:

- loads agent.env with the same safe KEY=VALUE rules;
- validates required Agent variables;
- derives HTTP(S) Server base URL from WS(S) URL;
- GET /readyz;
- GET local OAS /openapi.json;
- parses FOSTER_EMULATORS_JSON;
- verifies ADB executable;
- runs adb -s <serial> get-state for each configured emulator;
- validates oasConfigName/emulatorCode/adbSerial are present;
- exits non-zero if a required check fails.

It does not start or stop emulators and does not mutate game state.

## Deployment Docs

Add:
- how to trigger the package workflow;
- unzip to C:\FosterAgent;
- copy agent.env.example -> agent.env;
- run Test-FosterHost.ps1;
- run Start-FosterAgent.ps1;
- install login-start scheduled task only after smoke passes.

## Final Gate

No dedicated push CI.
The normal Foster Platform CI syntax-checks all deploy/windows/*.ps1 files.
