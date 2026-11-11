# Bot Navigation System

Server-side 2.5D walkability (terrain slopes, walls, ledges, object floors,
deep water), pathfinding, and client-side wall-slide steering so bots and
monsters stay on walkable ground and route around buildings, cliffs and
water instead of walking through them. Spans both repositories (`rose-offline` server, `rose-offline-client`).

## Table of Contents

1. [Why two halves are needed](#1-why-two-halves-are-needed)
2. [Walkability data sources](#2-walkability-data-sources)
3. [Nav grid bake (server)](#3-nav-grid-bake-server)
4. [Pathfinding and destination queries](#4-pathfinding-and-destination-queries)
5. [Server steering systems](#5-server-steering-systems)
6. [Client chase steering](#6-client-chase-steering)
7. [Configuration and validation](#7-configuration-and-validation)
8. [Known limitations](#8-known-limitations)

---

## 1. Why two halves are needed

Server movement is lerp-toward-destination with no colliders
(`rose-offline-server/src/game/systems/update_position_system.rs`), so without
routing, bots pick a goal and walk straight at it through geometry. But fixing
only the server is invisible:

- The client **simulates remote entities** (bots, monsters, NPCs) from packets:
  `AttackEntity` re-inserts an attack intent every tick, and the client's own
  `command_system` recomputes the chase destination straight at the target
  every frame. Server chase legs are overwritten locally.
- Remote entities have **no wall collision** client-side:
  `collision_player_system` only processes `With<CollisionPlayer>` (the local
  player), so rendered chasers cross walls regardless of server positions.

Hence: the server owns authoritative routing (travel legs, positions, ranges),
the client keeps rendered chases hugging the same walls.

## 2. Walkability data sources

There is no `.mov` file in this engine. Walkability comes from the same files
the client builds its terrain and Rapier colliders from:

| File | Role |
|---|---|
| `.HIM` heightmap (per block) | terrain floor height and slope; row 0 is the block's north edge |
| `.IFO` cnst/deco instances + zone `.ZSC` tables + `.ZMS` meshes | object collision triangles: walls and walkable floors (bridges, decks, ramps, gate platforms) |
| `.IFO` BlockType 11 (`collision_objects`) | designers' invisible walls: `LIST_DECO_SPECIAL.ZSC` object 2 (`Collision01.zms`) per record |
| `.IFO` event objects + `EVENT_OBJECT.ZSC` | floors only (never block, like the client) |
| `.IFO` warp gates + `LIST_DECO_SPECIAL.ZSC` object 1 | keep-out box (walking in changes zone) |
| `.IFO` water planes | deep-water blocking |
| `.ZON` | grid geometry and event positions, **not** collision |

ZSC part rule (mirrors client `zone_loader/spawning/objects.rs`): a part takes
part in movement only with a collision shape, and grass-kind meshes (grass,
bushes, plants; client `WindSway::for_mesh_path`) are walk-through. A part is
**collidable** unless `HEIGHT_ONLY` and **moveable** (stood on) unless
`NOT_MOVEABLE`. Part transforms are relative to the object (parent links are
not followed, as in the client); ZMS vertices are meters, Z-up.

## 3. Nav grid bake (server)

`rose-data/src/nav_grid.rs` defines `NavGrid`: row-major cells holding the
reasons a cell is blocked (`NAV_BLOCK_*` bits, 0 = walkable) and the floor
height bots stand on. Free cells form 4-connected components; a component is
**anchored** when it holds a designer placed point (start, revive, NPC or
monster spawn). Out-of-bounds = blocked.

Input collection is `rose-data-irose/src/zone_nav.rs` (called block by block
from `zone_database.rs::load_zone`); the bake is pure geometry in
`rose-data/src/nav_bake.rs` (`bake_nav_grid`, a port of the earlier Python
`nav_model.py`). Grids bake on worker threads in batches while zones load.

1. **Terrain**: bilinear HIM height per 1 m cell; a 2.5 m terrain quad steeper
   than 62 degrees (any triangle of either diagonal split) blocks as
   `SLOPE` where the floor is terrain: cliffs, river and moat banks.
2. **Collision triangles** are rasterized (cell center inside, or within half
   a cell). Flat triangles of moveable parts are floor samples; collidable
   parts that are `NOT_MOVEABLE` (walls, trunks) and steep faces of any
   collidable part are walls.
3. **Surfaces**: every object cell keeps a stack (terrain + merged floors,
   lowest 6). A surface is standable with 180 cm headroom and no wall in its
   body band (floor + 50 cm to floor + 180 cm), so curbs and overhead roofs
   don't block.
4. **Reachability**: surfaces of 4-adjacent cells link when their heights
   differ by at most 50 cm + 50 cm per cell of run (ramps, stairs); the cell
   floor is its highest surface linked to open terrain. Decks reached by a
   ramp or gangway are walkable; wall walks, pillar tops and statue heads are
   not (`NO_FLOOR`, or `WALL` when a wall caused it).
5. **Ledges**: a cell next to an object floor more than a step higher is
   `LEDGE` (no stepping onto platforms from the side, no dropping off).
6. **Water** deeper than 100 cm above the floor is `WATER`; warp gate
   footprints + 150 cm are `WARP`.
7. Stored as `ZoneData.nav_grid` (`rose-data/src/zone_database.rs`) with
   floor heights and anchors. The synthetic ocean zone gets no grid.

Model constants are `NavBakeParams::default()` (1 m cells; slope, step and
body height as used by the earlier model). `rose-data` and `rose-data-irose`
build at `opt-level = 3` in dev (workspace `Cargo.toml`) so startup baking
stays fast.

Startup logs: one `Zone walkability baked for N zones in Xs` info line, and a
`Nav bake zone <id>: ...` debug line per zone (grid size, bake time, cells per
blocked reason, object floors, reachable surfaces, triangle / placement / mesh
counts). Missing meshes, missing terrain blocks or `missing CNST/DECO table`
warnings mean the bake is blind for part of that zone.

## 4. Pathfinding and destination queries

`NavGrid::find_path` (`nav_grid.rs`): A* over 8-neighbour cells, no corner
cutting through diagonal gaps, octile heuristic, components checked first so
an unreachable goal answers instantly, line-of-sight smoothing on `cell_cm/2`
samples. Start snaps to the nearest free cell within 10 m, goal within 16 m.
`find_path_vec3` waypoints stand on the floor height.

Destination queries used by the bot and monster AI:

- `reachable_point(from, to, snap)`: `to` if an entity at `from` can walk
  there, else the nearest such cell within `snap` (a stranded `from` accepts
  any anchored component). Used for field wander spots
  (`bot_find_monster_spawn`), town walks (`bot_town_life::walkable`),
  restock trips (`bot_field_life`), party formation slots
  (`bot_follow_party_leader`) and monster AI wander / retreat
  (`npc_ai_system`).
- `rescue_point(at, radius)`: nearest free cell of an anchored component.
  Used for startup bot spawns (`startup_bots_system`), monster spawns
  (`bundles/entity.rs`) and stranded-bot rescue.
- `is_stranded(x, y)`: on a blocked cell or in an unanchored pocket.
- `floor_height_world(x, y)`: `update_position_system` sets z from it for
  every server-driven mover (players keep the z their client reports).

Unit tests: `cargo test -p rose-data nav` (grid: open field, wall-with-gap,
snapping, unreachable, OOB, corner cutting; bake: flat floor heights, cliff
slope, wall vs curb, ramp to platform with ledges, deep vs shallow water,
stranded pit and rescue, warp keep-out).

## 5. Server steering systems

All in `rose-offline-server/src/game/bots/bot_navigation.rs`, gated by
`GameConfig::enable_bot_navigation` (default `true`; set false to restore
straight-line walking). Scheduled in `game_world.rs` PreUpdate:

```
update_position_system -> bot_navigation_system -> command_system
    -> bot_chase_steering_system -> bot_separation_system
    -> bot_rescue_stranded_system
```

- **`bot_navigation_system`**: rewrites `NextCommand::Move` orders with
  `target: None` (spawn runs, NPC visits, town wanders) into one waypoint leg
  per tick before `command_system` promotes/broadcasts them. Moves under
  600 cm go straight only when the straight line is walkable. The component
  `BotNavigation { goal, waypoints, leg }` detects external overwrites (a new
  bot order) vs continued following. On arrival the exact goal is restored as
  the destination so the standard `command_system` distance check can stop
  the entity. An unreachable goal is moved to the nearest spot the entity can
  reach (`reachable_point`, 20 m); with none, the entity stops instead of
  walking through a wall, off a cliff or into deep water.
- **`bot_chase_steering_system`**: runs after `command_system`, where chase
  moves (`Command::Move` with a live `target`, e.g. attack approach) are
  recomputed per tick. If a 3 m probe toward the destination is blocked, the
  leg is deflected by 30/60/90/135 degrees either way (wall slide); if every
  angle is blocked it escalates to a full A* route that replans when the
  target drifts more than 4 m from the stored goal. Rewritten legs are
  rebroadcast as `MoveEntity` so clients see the server path.
- **`bot_separation_system`**: soft 1 m separation between mobile non-player
  entities (capped 2 m/s), with sector-index writeback so visibility stays
  consistent. A push that would move an entity onto blocked ground is
  skipped.
- **`bot_rescue_stranded_system`**: once a second, bots (not vendors or LLM
  buddies) found stranded (`NavGrid::is_stranded`) for 4 s (12 s while
  attacking) are moved to the nearest anchored walkable cell within 60 m, or
  the zone start, by leaving and rejoining the zone like buddy-bot teleports.

Unit tests: `cargo test -p rose-offline-server bot_navigation` (probe clear,
deflection geometry, gap pass-through).

## 6. Client chase steering

`rose-offline-client/src/systems/command_system.rs::npc_chase_steering_system`
(registered in `lib.rs` between `command_system` and
`update_position_system`):

- Matches every non-player mover (`Without<PlayerCharacter>`,
  `Or<(With<CharacterModel>, With<NpcModel>)>`), skips flying and sailing
  entities.
- For active `Command::Move`, probes the live Rapier world with the same 0.4 m
  ball cast the player wall check uses (`COLLISION_FILTER_COLLIDABLE`,
  excluding terrain and physics toys), deflecting the destination through the
  same angle ladder as the server.
- The local player is deliberately untouched — its movement is
  collision-clamped separately in `collision_player_system`, and the server
  echo must not be altered.
- It is a separate system because `command_system` is already at Bevy 0.19's
  16-parameter system-function limit; adding `ReadRapierContext` to it breaks
  scheduling elsewhere (see `pitfalls/ecs-system-tuples.md`).

Travel legs arriving from the server pass through this pass untouched: their
waypoints are chosen to keep the 3 m probe clear.

## 7. Configuration and validation

- **Toggle**: `GameConfig::enable_bot_navigation`
  (`rose-offline-server/src/game/resources/game_config.rs`).
- **Maps**: `rose-nav-map --data-idx <path/to/data.idx> [--zone <id>]...
  [--output nav-maps]` (crate `rose-offline-tools/rose-nav-map`) bakes every
  zone exactly like the server and writes one north-up PNG per zone (1 px per
  cell) plus `summary.txt` with the share of cells per blocked reason.
  Walkable ground is shaded by height; cyan = stranded pockets, red = wall,
  orange = slope, yellow = ledge, purple = no floor, blue = deep water,
  magenta = warp, black = outside the zone; markers for start, revive points,
  NPCs and spawns.
- **Startup**: grep server stdout for `Zone walkability baked` and (debug)
  `Nav bake zone`.
- **Live**: `/bot <n>` near a moat, river bank or cliff edge; bots should keep
  to the walkable side and cross by bridges and ramps.
- **Tests**: `cargo test -p rose-data nav`,
  `cargo test -p rose-data-irose zone_nav` and
  `cargo test -p rose-offline-server bot_navigation`.

## 8. Known limitations

- Step height (50 cm) and body height (180 cm) are assumptions carried over
  from the earlier Python model; the 62 degree slope limit was calibrated on
  the original client. Tune `NavBakeParams` if bots refuse real stairs or
  climb things players cannot.
- One floor per cell: where a walkable deck spans a walkable path below it,
  the deck wins and walking underneath is lost (a detour, never a wedge).
- Chase/pickup/follow legs under 600 cm travel straight unless blocked (by
  design: they are recomputed every tick and the probe catches actual walls).
- The grid never changes at runtime: destroyed/placed objects are not
  re-baked (zone statics don't move, so this is fine today).
- Synthetic ocean zone has no grid (ships use their own navigability checks,
  see `docs` sailing notes in the client repo).
