# Foster Platform Onboarding + H5 Implementation Plan

**Goal:** Make the existing foster platform operable for the first real customer without adding payment or a large admin frontend.

## Scope

1. Seed three MVP plans:
   - BASIC_AUTO_FOSTER / USER_FRIEND
   - PLATFORM_FISH / PLATFORM + FISH
   - PLATFORM_TAIKO_JADE / PLATFORM + TAIKO_JADE

2. Admin onboarding API:
   - POST /admin/onboard
   - Bearer token from FOSTER_ADMIN_TOKEN
   - input: customerId, planCode, serviceDays, loginTtlMinutes
   - create PENDING GameAccount
   - create PENDING_LOGIN Subscription with plan snapshot
   - allocate emulator capacity + create LoginSession
   - create service share link
   - return login/service URLs and subscription number
   - compensate partially-created rows if onboarding fails

3. Login confirmation integration:
   - successful confirmation activates PENDING_LOGIN subscriptions
   - first next_run_at = confirmation time

4. H5:
   - /login/{public_token}: QR/login/confirm page; control token lives in URL fragment
   - /service/{public_token}: service dashboard; control token lives in URL fragment
   - service page works read-only without fragment
   - controls disabled without control token
   - no neon/cyberpunk styling; quiet professional dark UI

5. Runnable Server:
   - DATABASE_URL
   - FOSTER_AGENT_TOKEN
   - FOSTER_BIND_ADDR default 0.0.0.0:8080
   - tracing
   - axum::serve

## Safety

- Admin endpoint fails closed if FOSTER_ADMIN_TOKEN is missing.
- Admin token is never returned or logged.
- Raw login/share tokens are returned only at creation.
- DB stores hashes only.
- Control token is in browser fragment for H5 entry URL, so it is not sent in the initial page request.
- Public page token remains read-only.
- Onboarding validates active plan and positive service duration.
- Subscription is not ACTIVE before successful login confirmation.

## Tests

- default plans exist after migration.
- onboarding creates account/subscription/login/share link.
- onboarding subscription is PENDING_LOGIN.
- successful login confirm activates subscription and sets next_run_at.
- unknown plan rejected without orphan account.
- no emulator capacity rejected without orphan account.
- admin auth rejects missing/wrong token.
- H5 routes render HTML without exposing control token in HTML.
- service H5 uses public token from pathname and control token from fragment.
- server config/main compile.

## Final Gate

One CI trigger after implementation:
fmt / check / all tests / clippy / Python compileall / provider-target test.
