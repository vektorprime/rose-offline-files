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

## What Is Left To Do

### A) Finish behavior implementation
1. Add and finish **heal behavior** module (`bot_use_heal_skill.rs`) and wire into thinker priority.
2. Validate and finalize chat/NPC visit scorer thresholds and action flow.
3. Ensure regular bot thinker ordering is sensible (revive/join/accept-party/combat/heal/buff/lively actions).

### B) Startup auto-spawn of regular bots
1. Add startup bot system for regular bots only (spawn on launch in map/zone).
2. Wire startup system into schedule after zone startup.
3. Add/finish startup bot config fields in `GameConfig` (count, zone, level range, behavior profile).
4. Update server main config wiring (`main.rs`) to feed startup bot settings.

### C) Party auto-accept guarantee
1. Finalize a **regular-bot** auto-accept system that accepts pending invites promptly.
2. Keep this independent of all LLM buddy systems.

### D) Stabilize and verify
1. Resolve compile errors from in-progress edits in regular bot files.
2. Run `cargo build` (no release, no clean) and fix all compilation errors in touched code.
3. Update this plan with final status when implementation is complete.

## Notes
- Diagnostic noise is currently present from `libs/big-brain` examples/tests in editor diagnostics; implementation focus remains the regular server code path.
- No LLM buddy bot behavior should be changed as part of this task.
