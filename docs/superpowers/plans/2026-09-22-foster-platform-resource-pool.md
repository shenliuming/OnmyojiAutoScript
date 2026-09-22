# Foster Platform Resource Pool Implementation Plan

> **Execution:** Continue on `feat/foster-platform`. Development commits do not trigger CI. Run one final full CI by updating `.github/ci-trigger/foster-platform`.

**Goal:** Make PLATFORM foster subscriptions executable with race-safe provider/resource allocation, explicit provider targeting in OAS, and correct resource-slot lifecycle.

**Architecture:** Server remains authoritative for resource inventory and allocation. Dispatcher reserves one provider resource cycle before Agent dispatch. Agent passes the chosen provider alias/resource type to the existing localhost OAS bridge. OAS selects only that provider row, verifies the card type, and executes one foster. Success confirms the allocation; failure releases it. BASIC remains unchanged.

## Global Rules

- BASIC `USER_FRIEND` behavior must remain unchanged.
- PLATFORM never lets OAS freely choose a provider.
- Server is the only resource allocation authority.
- One job has at most one live allocation.
- Slot reservation and release are transactional and idempotent.
- Near-expiry resource cycles are not allocatable.
- Friend relationships are manually maintained in MVP.
- No password/credential storage is introduced.
- Do not trigger GitHub Actions during intermediate commits.

## Task 1 — Add resource pool schema and domain

Create migration `0004_resource_pool.sql`.

### provider_account
- id
- provider_code UNIQUE
- game_uid nullable
- nickname
- provider_alias
- server_name nullable
- status: ACTIVE / DISABLED
- timestamps

### foster_friend_binding
- id
- game_account_id FK
- provider_account_id FK
- status: VERIFIED / SUSPECT / BROKEN / DISABLED
- verified_at
- last_failure_at
- last_failure_code
- timestamps
- UNIQUE(game_account_id, provider_account_id)

### foster_resource_cycle
- id
- provider_account_id FK
- resource_type: FISH / TAIKO_JADE
- resource_level
- start_at
- end_at
- slot_capacity
- occupied_slots
- status: AVAILABLE / FULL / ENDED / DISABLED
- timestamps
- indexes on (resource_type, status, end_at) and provider/status

### foster_resource_allocation
- id
- job_id FK
- resource_cycle_id FK
- provider_account_id FK
- status: RESERVED / CONFIRMED / RELEASED / EXPIRED
- reserved_at
- confirmed_at
- released_at
- occupied_until nullable
- release_reason nullable
- timestamps
- generated active-job guard UNIQUE for RESERVED/CONFIRMED

Add nullable history fields to `foster_job`:
- provider_account_id
- resource_cycle_id
- resource_allocation_id

Create `domain/resource.rs` with:
- ProviderStatus
- FriendBindingStatus
- ResourceCycleStatus
- ResourceAllocationStatus

## Task 2 — ResourcePoolService allocator

Create:
- `server/src/resource_pool/mod.rs`
- `repository.rs`
- `service.rs`

Default `min_remaining_minutes = 330`, configurable by environment.

Allocation algorithm:

1. Start transaction and lock job.
2. If live allocation already exists for job, return it unchanged.
3. Require PLATFORM subscription + resource_type.
4. Query candidates:
   - friend binding VERIFIED
   - provider ACTIVE
   - cycle AVAILABLE
   - matching resource_type
   - `end_at >= now + min_remaining`
   - `occupied_slots < slot_capacity`
5. Sort:
   - end_at ASC
   - occupied_slots DESC
   - cycle id ASC
6. Reserve candidate with conditional UPDATE:
   `occupied_slots = occupied_slots + 1`
   and switch cycle to FULL if capacity reached.
7. Insert RESERVED allocation.
8. Attach provider/cycle/allocation ids to job.
9. Commit.

If no candidate:
- set job WAITING_RESOURCE
- clear active execution ownership naturally by job status
- return WaitingResource.

## Task 3 — Allocation lifecycle

### Success
On `FosterSucceeded` for PLATFORM:
- verify identity first
- confirm RESERVED allocation
- set `occupied_until = completed_at + remaining_seconds`
- fallback to subscription interval when OCR remaining duration is unavailable
- keep cycle occupied count unchanged
- then complete job success.

### Failure
On execution failure:
- RELEASE RESERVED allocation
- decrement cycle occupied_slots exactly once
- FULL cycle becomes AVAILABLE if still valid
- PROVIDER_NOT_FOUND marks friend binding SUSPECT
- then use existing Scheduler failure policy.

### Reaper
Every resource scheduler tick:
- expire RESERVED allocations whose cycle ended
- expire CONFIRMED allocations whose `occupied_until <= now`
- decrement occupied_slots exactly once
- mark ended cycles ENDED
- reopen FULL cycle to AVAILABLE when a slot is released and cycle remains valid.

## Task 4 — Re-enter WAITING_RESOURCE / RETRY / WAITING_EMULATOR jobs

Fix scheduler recovery normalization:
- candidate selection already filters retry due time.
- before claim, normalize eligible:
  - WAITING_EMULATOR
  - WAITING_RESOURCE
  - RETRY
  - DEFERRED_* when due
  back to PENDING after gate checks.
- then claim emulator normally.

This ensures resource availability or host recovery can resume an existing nonterminal job.

## Task 5 — Integrate allocator into FosterDispatchService

For BASIC:
- unchanged.

For PLATFORM:
1. call ResourcePoolService.reserve_for_job()
2. if no resource → WAITING_RESOURCE and do not dispatch Agent
3. if reserved:
   - provider_alias from ProviderAccount
   - resource_type from allocation/cycle
   - include both in ExecuteFosterCommand
4. Agent/OAS executes.

Do not re-reserve when the same job already owns a RESERVED allocation.

## Task 6 — OAS specified provider selection

Add a PLATFORM branch to the one-shot OAS execution only.

### Provider matching
- Add broad friend-name OCR rule for visible rows.
- Detect visible resource cards with existing `order_targets.find_everyone()`.
- OCR visible friend names with boxes.
- Normalize provider alias and OCR text:
  - trim whitespace
  - lowercase latin
  - remove common punctuation/spaces
- Pair nickname OCR box and card area by nearest vertical center within a safe threshold.
- Only click a card on the row whose OCR name matches the Server-specified provider alias.
- Scroll using existing `perform_swipe_action()` up to bounded attempts.

### Resource validation
After clicking the selected provider card:
- call existing `check_card_num()`
- FISH must map to OAS `斗鱼`
- TAIKO_JADE must map to OAS `太鼓`
- mismatch → PROVIDER_NOT_FOUND
- provider absent after bounded scan → PROVIDER_NOT_FOUND

Then continue the existing enter-realm / no-slot / place-shikigami logic.

No changes to BASIC `_select_optimal_resource_card()`.

## Task 7 — Tests

### Resource allocator
- only VERIFIED friend bindings are candidates
- near-expiry cycle is rejected
- earliest expiry wins
- fuller cycle wins on equal end time
- capacity cannot oversell under concurrent reservations
- repeated reserve for same job is idempotent
- no resource → WAITING_RESOURCE

### Lifecycle
- success confirms allocation
- failure releases and decrements slot
- duplicate failure does not double-decrement
- PROVIDER_NOT_FOUND marks friend binding SUSPECT
- confirmed allocation expires and frees slot
- FULL cycle reopens after release

### Scheduler recovery
- WAITING_RESOURCE can resume after resource appears
- WAITING_EMULATOR can resume after host/emulator recovers
- RETRY resumes when retry_after is due

### Dispatcher
- PLATFORM command contains exact provider_alias/resource_type
- BASIC command remains provider_alias = None
- stale attempt events do not mutate allocation

### OAS pure tests where practical
- alias normalization
- OCR-name/card-row pairing
- resource-type mapping
- provider not found outcome

## Final CI Gate

One final CI trigger after implementation:
- cargo fmt --all --check
- cargo check --workspace
- cargo test --workspace --all-targets
- cargo clippy --workspace --all-targets -- -D warnings
- Python compileall for modified bridge/Kekkai files

## Phase 6 Completion Gate

- PLATFORM job gets a Server-selected provider.
- Slots cannot be oversold.
- Near-expiry cycles are excluded.
- OAS receives and targets one provider alias instead of free-selecting.
- Resource type is verified in game UI.
- Success CONFIRMS allocation and preserves occupied slot until foster expiration.
- Failure RELEASES allocation and frees slot exactly once.
- Provider-not-found marks friend binding suspect.
- WAITING_RESOURCE resumes when a resource becomes available.
- BASIC path remains unchanged.
- Final full CI passes.
