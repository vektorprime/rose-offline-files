# Regular Bot Expansion Plan (No LLM Buddy Changes)

## Scope Guardrail
- **Do not touch LLM buddy code paths** (`llm_buddy_*`, LLM API bot systems, LLM storage/manager, etc.).
- Focus only on **regular normal bots** under `rose-offline-server/src/game/bots` and regular server startup/config wiring.

## What Has Been Done So Far

### 1) Analysis and design
- Reviewed existing regular bot architecture in `rose-offline-server/src/game/bots` and related systems.
- Confirmed existing behaviors already include: combat targeting, revive, party invite acceptance action, buff skill usage, item pickup, and monster-spawn roaming.
- Confirmed existing XP/level flow exists, but there is no regular-bot auto-progression spending loop for new stat/skill points.

### 2) Tracking doc created
- Created this issue tracking file in `plans/bot-expansion-issue.md`.

### 3) In-progress code changes (regular bot path only)
- Updated `rose-offline-server/src/game/bots/create_bot.rs`:
  - Added reusable helper `spend_skill_points_with_bundle(...)`.
  - Kept `spend_skill_points(...)` and routed it through the new helper.
- Updated `rose-offline-server/src/game/bots/mod.rs` (in progress, not finalized yet):
  - Began integrating a configurable Big Brain profile (`BotBehaviorConfig`).
  - Began wiring additional regular-bot systems (auto-party-accept + auto-progression).
  - Began adding new behavior modules to the regular bot plugin.
- Added new regular bot modules:
  - `rose-offline-server/src/game/bots/bot_chat.rs` (ambient local chat behavior)
  - `rose-offline-server/src/game/bots/bot_visit_npc.rs` (NPC visit/walk behavior)

## Final Status: Completed

### A) Behavior Implementation
- [x] Added and finished **heal behavior** module (`bot_use_heal_skill.rs`) and wired into thinker priority.
- [x] Validated and finalized chat/NPC visit scorer thresholds and action flow.
- [x] Ensured regular bot thinker ordering is sensible (revive/join/accept-party/combat/heal/buff/lively actions).

### B) Startup Auto-Spawn
- [x] Implemented startup bot system for regular bots to spawn on launch in map/zone.
- [x] Wired startup system into the schedule after zone startup.
- [x] Added startup bot configuration fields to `GameConfig` (count, zone, level range, behavior profile).
- [x] Updated server main config wiring in `main.rs` to feed startup bot settings.

### C) Party Auto-Accept
- [x] Finalized a **regular-bot** auto-accept system that accepts pending invites promptly.
- [x] Verified independence from LLM buddy systems.

### D) Stabilization and Verification
- [x] Resolved all compilation errors in regular bot files.
- [x] Verified with `cargo build` that the project compiles successfully.

## Notes
- Diagnostic noise is currently present from `libs/big-brain` examples/tests in editor diagnostics; implementation focus remains the regular server code path.
- No LLM buddy bot behavior should be changed as part of this task.
