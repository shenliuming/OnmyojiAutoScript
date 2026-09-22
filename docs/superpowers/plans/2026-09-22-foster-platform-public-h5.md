# Foster Platform Public H5 / Pause / Quiet Period Implementation Plan

**Goal:** Provide a secure public status API and control API for one subscription using opaque view/control tokens. Tokens are never stored raw.

## Scope

- `GET /r/{public_token}`: service status, plan/resource type, expiry, today success count, next run, pause state, quiet periods, recent jobs, relogin indicator.
- `POST /r/{control_token}/pause`: presets `1H`, `2H`, `4H`, `TODAY`.
- `DELETE /r/{control_token}/pause`: clear manual pause.
- `PUT /r/{control_token}/quiet-periods`: replace enabled quiet periods atomically.
- Share link service creates strong random public/control tokens and stores SHA-256 only.
- Public token cannot mutate controls.
- Revoked/expired link returns 404/410.
- MVP timezone for quiet-period editing is `Asia/Shanghai`.
- No frontend framework in this phase; APIs are the H5 backend contract.

## Data

Create `foster_share_link`:

- id
- subscription_id
- public_token_hash CHAR(64)
- control_token_hash CHAR(64)
- status ACTIVE / REVOKED
- expire_at nullable
- last_access_at
- access_count
- created_at / updated_at

One active link per subscription is enough for MVP; regenerating revokes old link and creates a new row.

## Public Status

Return:

- subscriptionNo
- serviceStatus
- characterName / serverName
- loginStatus / verifyStatus
- planName
- resourceMode / resourceType
- dailyTargetRuns
- todaySuccessCount
- lastSuccessAt
- nextRunAt
- manualPauseUntil
- effectiveBlockedUntil
- serviceEndAt
- reloginRequired
- quietPeriods[]
- recentJobs[] (max 20)

No internal IDs, host IDs, emulator IDs, provider IDs, token hashes, or account secrets.

## Control

Pause preset:
- 1H / 2H / 4H = now + duration.
- TODAY = end of current day in Asia/Shanghai.
- Pause only moves `manual_pause_until` forward; a shorter request cannot shorten an existing later pause.
- DELETE explicitly clears it.

Quiet periods:
- 0..8 entries.
- weekdayMask 1..127.
- `HH:MM` start/end.
- buffer 0..120 minutes.
- timezone fixed to Asia/Shanghai in MVP.
- Replace in one transaction.

## Tests

- raw token is not stored.
- public token reads but cannot control.
- wrong token 404.
- revoked token rejected.
- expired token 410.
- status counts only SUCCESS today.
- pause 2H defers scheduler gate.
- shorter pause does not shorten existing pause.
- clear pause resumes eligibility.
- quiet periods replace atomically.
- invalid quiet period rejected.
- response does not expose internal IDs/token hashes.

## Final Gate

Development commits do not trigger Actions. At phase end update `.github/ci-trigger/foster-platform` once and require fmt/check/test/clippy/Python compileall green.
