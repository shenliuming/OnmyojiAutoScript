# Frontend / Backend Separation Design

Date: 2026-09-26

## Goal

Make the foster/login tool understandable and operable as a conventional frontend plus backend application. A user should launch it once, see separate frontend/backend consoles with useful status and errors, open one web address, and use the same frontend for the admin console and customer QR/login pages. Keep the existing Agent, emulator and OAS boundaries intact.

## Current state and constraints

- `foster-server` is an Axum/Rust service on port 8080. It currently serves admin/login/customer HTML directly from Rust handlers as well as JSON APIs and the Agent WebSocket.
- `foster-agent` runs on the Windows machine and talks to the server and local OAS/ADB/emulators.
- MySQL is currently provided by Docker Compose. Separating the frontend from the backend does not by itself remove this database dependency; the first implementation must keep database startup explicit and visible in the backend console/status rather than silently claiming Docker is gone.
- NATAPP is the external entry point. Customer pages must continue to work over that entry point, and the Agent WebSocket must remain proxied to the backend.
- The automatic post-QR identity OCR path is incomplete (`/login/detect` is missing); this design must not imply that frontend separation completes QR login.
- Do not modify the OAS game script/source as part of this architecture change.

## Recommended architecture

Create a standalone `web/` frontend package using TypeScript and Vite. Move the admin dashboard and customer login/service pages into frontend routes (`/admin`, `/login/:token`, and `/service/:token`). The Rust backend becomes API/WebSocket-only for browser traffic, retaining `/admin/*` operations, `/public/login/*`, `/r/:token` status/control API operations, health/readiness routes, and `/agent/ws`.

During local operation, Vite serves the frontend on `127.0.0.1:5173` and proxies backend paths plus `/agent/ws` to `127.0.0.1:8080`. This gives browser code one origin, avoids browser CORS setup, and lets NATAPP expose only the frontend port while forwarding API and WebSocket traffic through Vite. Do not expose OAS port 22270 or the database port publicly.

The Windows launcher starts the database prerequisite, Rust backend and frontend as distinct visible PowerShell windows. Its own small status window prints links, readiness checks, and which component failed. It opens the frontend only after the backend readiness endpoint and frontend HTTP route respond. Backend and frontend windows remain open to show logs. Agent/OAS remain separately observable and are not silently conflated with either web process.

For deployment, build the frontend to static assets and run the same frontend proxy-capable server configuration (or a small static/proxy host) behind NATAPP. The local launch path and public path must use the same API paths and WebSocket behavior; do not maintain separate customer-login implementations.

## Request/data flow

1. Browser loads `/admin`, `/login/:token`, or `/service/:token` from the frontend process.
2. Frontend calls existing JSON endpoints using relative URLs; the frontend proxy forwards them to Rust.
3. The Agent connects directly to Rust at `/agent/ws` and continues using local OAS/ADB configuration.
4. Login state/events and QR images flow from Rust to the frontend over existing public endpoints/SSE; the customer browser never talks directly to an emulator or OAS.
5. Admin token stays in the URL fragment/client memory and is sent as the existing bearer token only to backend admin APIs.

## Startup and failure behavior

- Each process prints its working directory, selected port, readiness URL, and actionable startup errors; never print secret tokens.
- Detect port conflicts before launching and identify which process/port blocked startup.
- Frontend may start while backend initializes, but launcher must not report ready or open the final page until both are reachable.
- Missing Node/npm, Rust toolchain/binary, database, NATAPP executable/config, or Agent config must produce a clear phase-specific error and keep the relevant console open.
- Start/stop should be idempotent where possible; do not terminate unrelated Node, Rust, Docker, or emulator processes by process name.
- External tunnel readiness must distinguish local frontend readiness from public NATAPP reachability.

## Scope boundaries

Included: frontend package and routes, backend API-only browser contract, local frontend proxy including SSE/WebSocket, visible per-component startup windows/status, and NATAPP routing documentation/configuration.

Not included: redesigning customer UI, changing Agent protocol, fixing the missing OAS OCR endpoint, modifying OAS game scripts, replacing MySQL, or automatically installing system software. OCR remains a separate later delivery phase.

## Acceptance criteria

1. One launcher starts frontend and backend without requiring the user to manually open terminals or type commands; their logs remain visible in separate windows.
2. `http://127.0.0.1:5173/admin` renders the admin page and can call authenticated admin APIs through the proxy.
3. A customer login URL and service URL render in the frontend; QR, status polling/SSE, platform selection and identity submission reach the existing backend APIs.
4. Agent WebSocket proxy works through the frontend origin, and direct local Agent-to-backend remains supported.
5. NATAPP configured to the frontend works for a remote browser, including API, SSE and WebSocket; no backend/OAS/database port is exposed publicly.
6. Missing prerequisites and occupied ports show actionable errors in the matching visible console, without a false “ready” message.
7. Existing server/Agent behavior and database migrations remain intact. OCR completion is explicitly not claimed by this milestone.

## Risks and decisions

- The server currently combines page paths and API paths. Route migration must preserve browser endpoint semantics while moving only HTML responses; route/path collisions require explicit proxy allowlists.
- Vite development mode is not a production server. The launcher must use a built frontend/preview or an explicitly supported production static/proxy mode; it must not quietly expose the dev server as a stable public deployment.
- Docker currently supplies MySQL. A later implementation decision must either keep the database prerequisite visible/managed or separately approve and implement a native database alternative. Frontend/backend separation alone cannot remove it.
- The login OCR endpoint is still absent and remains a release blocker for complete post-scan identity verification.
