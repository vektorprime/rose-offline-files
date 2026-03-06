# Pitfalls and Lessons Learned

This file documents issues encountered during development and their solutions to prevent repeating the same mistakes.

---

## Bot Combat Sync Issue (March 2026)

### Problem
When an LLM-controlled bot was assigned to attack a monster:
1. Monster didn't appear to take damage on the client side
2. Bot would disappear after a while during combat

### Root Causes

#### 1. Missing Zone Filter in Entity Lookup
**Issue:** The Attack command entity lookup in `llm_buddy_bot_system.rs` found entities by ClientEntityId but didn't filter by zone. This caused it to find entities in different zones, resulting in massive distance discrepancies (MCP reported 1886, command system saw 35390).

**Fix:** Added zone filtering to ensure target entity is in the same zone as the bot:
```rust
// Before (broken):
.find(|(_, client_entity, _, _, _, _, _, _)| client_entity.id.0 == target_entity_id as usize)

// After (fixed):
.find(|(_, client_entity, pos_opt, _, _, _, _, _)| {
    client_entity.id.0 == target_entity_id as usize &&
    pos_opt.map_or(false, |pos| pos.zone_id == bot_zone)
})
```

**File:** `rose-offline-server/src/game/systems/llm_buddy_bot_system.rs`

#### 2. Attack Range Threshold Too Strict
**Issue:** The attack range check was too strict, causing the bot to constantly switch to Move command without actually moving.

**Fix:** Used 3x attack range as effective range threshold to allow bot to start moving toward target earlier:
```rust
let effective_attack_range = attack_range * 3.0;

if effective_attack_range < distance {
    // Switch to Move command
}
```

**File:** `rose-offline-server/src/game/systems/command_system.rs`

### Key Takeaways
1. **Always filter by zone** when looking up entities by ClientEntityId - the same ID can exist in multiple zones
2. **Distance calculations** between entities are meaningless if they're in different zones
3. **Diagnostic logging** is essential for tracking down entity lookup issues - log the zone along with entity IDs

---
