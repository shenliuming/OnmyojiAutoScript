# Foster Platform Resource Admin Implementation Plan

**Goal:** Make PLATFORM resource operations manageable without direct SQL.

## Scope

1. Reuse existing `FOSTER_ADMIN_TOKEN`.
2. Admin APIs:
   - POST `/admin/providers`
   - PUT `/admin/providers/{provider_id}/status`
   - PUT `/admin/accounts/{game_account_id}/providers/{provider_id}`
   - POST `/admin/providers/{provider_id}/cycles`
   - PUT `/admin/resource-cycles/{cycle_id}/status`
   - GET `/admin/resource-pool`
3. Provider create is idempotent by `provider_code`.
4. Friend binding upserts to VERIFIED/SUSPECT/DISABLED.
5. Cycle validation:
   - supported resource type
   - end_at > start_at
   - slot_capacity > 0
6. Cycle status only supports AVAILABLE/DISABLED.
7. Resource pool list shows provider + cycle + occupied/capacity + active friend-binding count.

## Safety

- All endpoints fail closed when admin token is missing.
- No passwords or game credentials accepted.
- Cannot create a cycle for a missing provider.
- Cannot verify a binding for a missing game account/provider.
- Duplicate provider alias returns conflict.
- Disabling a cycle must prevent new reservations but not destroy existing allocations.

## Tests

- auth missing/wrong token rejected;
- provider create and idempotent update;
- duplicate alias conflict;
- friend binding upsert;
- cycle create validation;
- disabled cycle not allocatable;
- resource pool listing shows occupancy/binding counts.

## Final Gate

One final CI trigger after implementation:
- cargo fmt --all --check
- cargo check --workspace
- cargo test --workspace --all-targets
- cargo clippy --workspace --all-targets -- -D warnings
- Python compileall
- provider targeting unittest
