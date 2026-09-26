# Frontend / Backend Separation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a conventional standalone web frontend and Rust API backend with visible local startup windows, preserving customer login pages, Agent connectivity, and NATAPP access.

**Architecture:** Add a TypeScript/Vite `web/` app for `/admin`, `/login/:token`, and `/service/:token`; preserve backend JSON/SSE/WebSocket APIs and proxy them through the frontend. Use a Windows launcher that starts frontend and backend in separate visible PowerShell windows and waits for both readiness checks before opening the UI.

**Tech Stack:** Rust, Axum, TypeScript, Vite, Windows PowerShell, existing MySQL/Docker Compose, NATAPP.

**Spec:** `foster-platform/docs/superpowers/specs/2026-09-26-frontend-backend-separation-design.md`

## Global Constraints

- Keep local frontend at `127.0.0.1:5173` and Rust backend at `127.0.0.1:8080`.
- Bind the browser-facing backend to `127.0.0.1:8080`; NATAPP exposes only the local frontend listener.
- Do not expose OAS port 22270 or MySQL publicly.
- Keep the Agent connection at `/agent/ws` supported directly and through the frontend proxy.
- Keep API paths `/admin/*`, `/public/login/*`, `/r/*`, `/healthz`, and `/readyz` stable.
- Keep QR/login and service/customer pages on one frontend implementation.
- Never print admin/agent secrets in any console or URL log.
- Do not modify OAS game scripts/source or claim the missing `/login/detect` OCR path is implemented.
- Do not kill unrelated processes by process name; report occupied ports precisely.

## Review Focus

- Deep-link refresh on `/login/:token` or `/service/:token` must return the SPA shell, not 404 (Task 1 route fallback smoke check).
- `/r/:token` must remain a backend JSON API and not be swallowed by SPA history fallback (Tasks 2 and 4 proxy/API routing checks).
- SSE and WebSocket proxy upgrades must remain streaming connections rather than buffered HTTP responses (Task 2 proxy checks).
- Admin token in URL fragment must not be forwarded in HTTP referrer/query/logs (Task 1 frontend auth check and Task 5 secret-log check).
- Missing MySQL/Docker/Node/Rust and port collisions must stop readiness with a component-specific explanation (Task 5 startup failure checks).

---

### Task 1: Create the standalone web application and migrate the admin page

**Files:**
- Create: `foster-platform/web/package.json`
- Create: `foster-platform/web/vite.config.ts`
- Create: `foster-platform/web/index.html`
- Create: `foster-platform/web/src/main.tsx`
- Create: `foster-platform/web/src/router.tsx`
- Create: `foster-platform/web/src/pages/AdminPage.tsx`
- Create: `foster-platform/web/src/lib/api.ts`
- Create: `foster-platform/web/src/lib/adminAuth.ts`
- Test: `foster-platform/web/src/lib/adminAuth.test.ts`
- Test: `foster-platform/web/src/router.test.tsx`

**Interfaces:**
- Consumes: current admin page behavior at backend `crates/server/src/onboarding/admin_page.rs`; API responses from `/admin/hosts`, `/admin/hosts/{host_id}/emulators`, and `/admin/onboard`.
- Produces: SPA routes `/admin`, `/login/:token`, `/service/:token`; `apiFetch(path: string, init?: RequestInit): Promise<Response>`; `readAdminToken(): string | null` that reads and removes the token fragment after storing it in tab-scoped `sessionStorage`.

- [ ] **Step 1: Scaffold the Vite/React TypeScript app and route unit tests.** Assert known paths map to the expected page component and unknown paths show a not-found state.
- [ ] **Step 2: Run frontend tests to verify the route tests fail before implementation.** Run: `npm test -- --run` from `foster-platform/web`. Expected: missing route module/test failure.
- [ ] **Step 3: Implement the router and API/auth utilities.** Admin authorization uses `Authorization: Bearer <token>` and never adds the token to a request URL; read/remove only the `#token=` fragment and store it in tab-scoped `sessionStorage` so route reloads continue to work without persisting across browser sessions.
- [ ] **Step 4: Migrate the admin dashboard UI.** Preserve host/emulator status, onboard-link creation, clipboard copy, and customer/service links; use `apiFetch` for calls.
- [ ] **Step 5: Add unit coverage for token-fragment handling and admin API headers.** Assert query strings/referrers are not used for secrets and absent token produces an actionable admin-auth prompt.
- [ ] **Step 6: Run frontend tests and production build.** Run: `npm test -- --run` and `npm run build`. Expected: PASS and generated `web/dist/index.html` plus hashed assets.

### Task 2: Add frontend proxy configuration and preserve streaming APIs

**Files:**
- Modify: `foster-platform/web/vite.config.ts`
- Create: `foster-platform/web/src/lib/proxyPaths.ts`
- Test: `foster-platform/web/src/lib/proxyPaths.test.ts`
- Modify: `foster-platform/web/README.md`

**Interfaces:**
- Consumes: frontend route map from Task 1; backend local origin `http://127.0.0.1:8080`.
- Produces: Vite `server.proxy` and `preview.proxy` for `/admin/*`, `/public/login/*`, `/r/*`, `/healthz`, `/readyz`, and `/agent/ws`; SPA shell fallback applies only to frontend routes and asset requests.

- [ ] **Step 1: Write proxy-path classification tests.** Assert exact API families are proxied, `/admin` page stays frontend-owned, `/login/:token` and `/service/:token` stay frontend-owned, and `/r/:token` is proxied.
- [ ] **Step 2: Run proxy-path tests and verify expected failures.** Run: `npm test -- --run src/lib/proxyPaths.test.ts` from `foster-platform/web`.
- [ ] **Step 3: Implement path classification and Vite proxy rules.** Configure WebSocket proxying for `/agent/ws`; retain streaming response semantics for SSE; use `changeOrigin: false` and local backend target.
- [ ] **Step 4: Document local ports and proxy contract.** State that frontend listens on `127.0.0.1:5173`, backend on `127.0.0.1:8080`, and external NATAPP must target the frontend listener only.
- [ ] **Step 5: Run tests and inspect Vite config using a local smoke fixture.** Expected: proxy table matches tested paths; API, SSE and websocket paths are not rewritten or swallowed by SPA fallback.

### Task 3: Move customer login and service pages into the frontend

**Files:**
- Create: `foster-platform/web/src/pages/LoginPage.tsx`
- Create: `foster-platform/web/src/pages/ServicePage.tsx`
- Create: `foster-platform/web/src/components/QrPanel.tsx`
- Create: `foster-platform/web/src/components/IdentityForm.tsx`
- Test: `foster-platform/web/src/pages/LoginPage.test.tsx`
- Test: `foster-platform/web/src/components/IdentityForm.test.tsx`
- Modify: `foster-platform/crates/server/src/app.rs`
- Modify: `foster-platform/crates/server/src/onboarding/mod.rs`
- Modify: `foster-platform/crates/server/src/onboarding/h5.rs`
- Test: `foster-platform/crates/server/tests/login_public_api.rs`

**Interfaces:**
- Consumes: existing public login APIs (`/public/login/:token`, `/meta`, `/qr`, `/events`, `/platform`, `/identity`, `/confirm`) and service APIs (`/r/:token`, `/r/:token/pause`, `/r/:token/quiet-periods`).
- Produces: customer pages rendered only by frontend routes; backend retains JSON/image/SSE APIs and drops only HTML page handlers after parity checks.

- [ ] **Step 1: Add frontend behavior tests for QR, status, platform selection, identity submission and verified confirmation.** Preserve current UX behavior: QR only image, Android/iOS choice synchronized with status, fields for searchable server/manual character/manual UID, confirm only after backend verifies identity.
- [ ] **Step 2: Run the new page tests to verify expected failures.** Run: `npm test -- --run src/pages/LoginPage.test.tsx src/components/IdentityForm.test.tsx` from `foster-platform/web`.
- [ ] **Step 3: Implement the login and service pages using the existing API contract.** Keep API calls relative so Vite/NATAPP proxy is same-origin; do not add an independent emulator screenshot display.
- [ ] **Step 4: Compare backend page handler behavior to frontend parity checklist.** Preserve all statuses, expiry/retry messages, polling/SSE reconnection, form validation and link expiry behavior before removing any HTML handler.
- [ ] **Step 5: Remove HTML-only backend routes/handlers after parity is proven.** Keep JSON/SSE/QR endpoints and Agent WebSocket routes unchanged; add server test asserting those API routes still return their existing content types.
- [ ] **Step 6: Run frontend tests/build and Rust server login API tests.** Run: `npm test -- --run && npm run build`; `cargo test -p foster-server --test login_public_api` from `foster-platform`. Expected: all PASS.

### Task 4: Implement separate visible backend and frontend startup windows

**Files:**
- Create: `foster-platform/deploy/windows/Start-FosterFrontend.ps1`
- Create: `foster-platform/deploy/windows/Start-FosterBackend.ps1`
- Modify: `D:\workspace\OnmyojiAutoScript\启动寄养系统.ps1`
- Modify: `D:\workspace\OnmyojiAutoScript\启动寄养系统.bat`
- Modify: `foster-platform/deploy/windows/README.md`
- Test: `foster-platform/deploy/windows/Test-FrontendBackendLauncher.ps1`

**Interfaces:**
- Consumes: compiled backend and frontend build from Tasks 1–3; current server env and database setup; local Agent startup remains a distinct process.
- Produces: launcher invocation `启动寄养系统.bat`; backend window starts only the MySQL service from `docker-compose.dev.yml`, waits for its health check, sets `DATABASE_URL=mysql://root:foster@127.0.0.1:3307/foster` plus required secret env vars without echoing values, binds `FOSTER_BIND_ADDR=127.0.0.1:8080`, and runs `cargo run -p foster-server`; frontend window runs Vite production preview at `127.0.0.1:5173`; launcher waits for `/readyz` and frontend `/admin` before opening the browser.

- [ ] **Step 1: Define launcher state/result model and write PowerShell smoke checks.** Cover available ports, missing project/runtime paths, database unavailable, and child-process startup failure.
- [ ] **Step 2: Run smoke checks against placeholder scripts and verify expected failure messages.** Run: `powershell -NoProfile -File deploy/windows/Test-FrontendBackendLauncher.ps1` from the foster repo.
- [ ] **Step 3: Implement backend starter.** Run `docker compose -f docker-compose.dev.yml up -d mysql`, wait for its health check, set the development database URL and tokens from the existing protected config without printing secret values, bind the server to `127.0.0.1:8080`, then run `cargo run -p foster-server`; print `http://127.0.0.1:8080/readyz`, preserve exit code, and leave window open on error.
- [ ] **Step 4: Implement frontend starter.** Require a successful production build, run preview bound to `127.0.0.1:5173`, print frontend URL and proxy target, leave window open on error.
- [ ] **Step 5: Update root launcher.** Start backend and frontend in two distinct PowerShell windows, start OAS/Agent only after backend readiness, poll each component with bounded timeout, open `http://127.0.0.1:5173/admin#token=...` only when web services are ready, and keep a concise coordinator status console.
- [ ] **Step 6: Run PowerShell parser and launcher smoke checks.** Run: `Parser::ParseFile` for all three scripts and the smoke-check script. Expected: no parser errors; actionable failures per fixture; no destructive process-name kills.

### Task 5: NATAPP configuration and end-to-end acceptance

**Files:**
- Modify: `foster-platform/deploy/README.md`
- Modify: `foster-platform/deploy/windows/README.md`
- Modify: `D:\workspace\OnmyojiAutoScript\LOGIN_FLOW_CURRENT.md`
- Test: `foster-platform/deploy/windows/Test-FosterFrontendProxy.ps1`

**Interfaces:**
- Consumes: frontend listener on port 5173, backend listener on 8080, existing NATAPP domain/config, login API and websocket paths from Tasks 1–4.
- Produces: validated local and public smoke procedure with clear separation between frontend/backend readiness and OCR/login completion.

- [ ] **Step 1: Add frontend proxy smoke checks.** Check `/admin` returns frontend HTML; `/readyz` reaches backend; `/public/login/<invalid>/meta` reaches backend JSON (not SPA); `/agent/ws` returns a WebSocket upgrade response for a test stub; SSE content type remains `text/event-stream`.
- [ ] **Step 2: Run local proxy smoke checks.** Run: `powershell -NoProfile -File deploy/windows/Test-FosterFrontendProxy.ps1`. Expected: per-route PASS/FAIL report.
- [ ] **Step 3: Update NATAPP target guidance.** Point tunnel only to frontend 5173; document API and WebSocket proxy behavior; explicitly verify public endpoint from an external browser/network before calling it available.
- [ ] **Step 4: Run end-to-end acceptance with MySQL, backend, frontend, Agent and one emulator.** Confirm admin host list, create a customer login URL, remote page loads QR and status updates, platform choice and identity fields reach backend, and public URL remains reachable.
- [ ] **Step 5: Record limitations in `LOGIN_FLOW_CURRENT.md`.** Mark OCR identity detection as incomplete until `/login/detect` exists and real server/character/UID OCR has been validated; do not count customer form display as OCR completion.
- [ ] **Step 6: Run final review commands.** Run frontend tests/build, targeted server login API tests, script parser/smoke checks, proxy smoke checks and `git diff --check`. Expected: all commands pass; document any runtime acceptance blocked by UAC, Docker/MySQL, NATAPP or emulator availability instead of reporting success.

## Execution notes

- Execute one task at a time and review its diff before the next task.
- The repository already has unrelated user changes. Stage/commit only files belonging to the task currently completed; never sweep the whole worktree into a commit.
- No database migration is required for frontend separation. Existing pending migration `0009_login_identity_submission.sql` must not be dropped or silently combined with this work.
- OCR implementation is not part of this plan and remains a separate release milestone.
