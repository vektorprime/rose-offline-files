use std::collections::HashMap;
use std::sync::Arc;

use bevy::math::{Quat, Vec2, Vec3, Vec3Swizzles};
use log::{debug, warn};

use rose_data::{
    NavGrid, NpcConversationId, NpcId, SkyboxId, StringDatabase, ZoneData, ZoneDatabase, ZoneEventObject,
    ZoneId, ZoneList, ZoneListEntry, ZoneMonsterSpawnPoint, ZoneNpcSpawn, WORLD_TICKS_PER_DAY,
    NAV_AGENT_RADIUS_CM,
};
use rose_file_readers::{
    stb_column, IfoEventObject, IfoFile, IfoMonsterSpawn, IfoMonsterSpawnPoint, IfoNpc,
    IfoReadOptions, StbFile, VfsFile, VfsPath, VfsPathBuf, VirtualFilesystem, ZmsFile, ZonFile,
    ZonReadOptions, ZscCollisionFlags, ZscFile,
};

const MIN_SECTOR_SIZE: u32 = 5000;
const MAX_SECTOR_SIZE: u32 = 12000;
const OCEAN_ZONE_ID: usize = 200;
const OCEAN_ZONE_ID_U16: u16 = 200;
const OCEAN_ZONE_FILE: &str = "3DDATA/MAPS/OCEAN/OCEAN.ZON";
const OCEAN_CNST_TABLE: &str = "3DDATA/MAPS/OCEAN/OCEAN_CNST.ZSC";
const OCEAN_DECO_TABLE: &str = "3DDATA/MAPS/OCEAN/OCEAN_DECO.ZSC";
const OCEAN_START_EVENT: &str = "start";
const OCEAN_REVIVE_EVENT: &str = "revive";
const OCEAN_SECTOR_SIZE: u32 = 5000;
const OCEAN_ZONE_NAME: &str = "Ocean";
const OCEAN_ZONE_DESCRIPTION: &str = "Open sailing waters.";
const OCEAN_PLANET: usize = 1;

pub struct StbZone(pub StbFile);

#[allow(dead_code)]
impl StbZone {
    pub fn rows(&self) -> usize {
        self.0.rows()
    }

    stb_column! { 1, get_zone_file, &str }
    stb_column! { 2, get_zone_start_event_position_name, &str }
    stb_column! { 3, get_zone_revive_event_position_name, &str }
    stb_column! { 0, get_zone_is_underground, bool }
    stb_column! { 5, get_zone_background_music_day, &str }
    stb_column! { 6, get_zone_background_music_night, &str }
    stb_column! { 7, get_zone_skybox_id, SkyboxId }
    stb_column! { 8, get_zone_minimap_filename, &str }
    stb_column! { 9, get_zone_minimap_start_x, u32 }
    stb_column! { 10, get_zone_minimap_start_y, u32 }
    stb_column! { 11, get_zone_deco_table, &str }
    stb_column! { 12, get_zone_cnst_table, &str }
    stb_column! { 13, get_zone_day_cycle_time, u32 }
    stb_column! { 14, get_zone_morning_time, u32 }
    stb_column! { 15, get_zone_day_time, u32 }
    stb_column! { 16, get_zone_evening_time, u32 }
    stb_column! { 17, get_zone_night_time, u32 }
    stb_column! { 18, get_zone_pvp_state, u32 }
    stb_column! { 19, get_zone_planet, u32 }
    stb_column! { 20, get_zone_footstep_type, u32 }
    stb_column! { 21, get_zone_camera_type, u32 }
    stb_column! { 22, get_zone_join_trigger, &str }
    stb_column! { 23, get_zone_kill_trigger, &str }
    stb_column! { 24, get_zone_dead_trigger, &str }
    stb_column! { 25, get_zone_sector_size, u32 }
    stb_column! { 26, get_zone_string_id, &str }
    stb_column! { 27, get_zone_weather_type, u32 }
    stb_column! { 28, get_zone_party_xp_a, u32 }
    stb_column! { 29, get_zone_party_xp_b, u32 }
    stb_column! { 30, get_zone_vehicle_use_flags, u32 }
    stb_column! { 31, get_zone_revive_zone_no, u32 }
    stb_column! { 32, get_zone_revive_pos_x, u32 }
    stb_column! { 33, get_zone_revive_pos_y, u32 }
}

pub enum LoadZoneError {
    NotExists,
    ZonFileInvalidPath,
    ZonFileNotFound,
}

fn create_monster_spawn(
    spawn: &IfoMonsterSpawnPoint,
    object_offset: Vec3,
) -> ZoneMonsterSpawnPoint {
    let transform_spawn_list = |spawn_list: &Vec<IfoMonsterSpawn>| {
        spawn_list
            .iter()
            .filter_map(|&IfoMonsterSpawn { id, count }| {
                NpcId::new(id as u16).map(|npc_id| (npc_id, count as usize))
            })
            .collect()
    };

    ZoneMonsterSpawnPoint {
        position: Vec3::new(
            spawn.object.position.x,
            spawn.object.position.y,
            spawn.object.position.z,
        ) + object_offset,
        basic_spawns: transform_spawn_list(&spawn.basic_spawns),
        tactic_spawns: transform_spawn_list(&spawn.tactic_spawns),
        interval: spawn.interval,
        limit_count: spawn.limit_count,
        range: spawn.range,
        tactic_points: spawn.tactic_points,
    }
}

fn create_npc_spawn(npc: &IfoNpc, object_offset: Vec3) -> Option<ZoneNpcSpawn> {
    Some(ZoneNpcSpawn {
        npc_id: NpcId::new(npc.object.object_id as u16)?,
        position: Vec3::new(
            npc.object.position.x,
            npc.object.position.y,
            npc.object.position.z,
        ) + object_offset,
        direction: Quat::from_xyzw(
            npc.object.rotation.x,
            npc.object.rotation.y,
            npc.object.rotation.z,
            npc.object.rotation.w,
        )
        .to_euler(bevy::math::EulerRot::XYZ)
        .2
        .to_degrees(),
        conversation: NpcConversationId::new(npc.quest_file_name.to_string()),
    })
}

fn create_event_object(
    event_object: &IfoEventObject,
    object_offset: Vec3,
    map_chunk_x: i32,
    map_chunk_y: i32,
) -> ZoneEventObject {
    ZoneEventObject {
        event_id: event_object.object.event_id,
        map_chunk_x,
        map_chunk_y,
        position: Vec3::new(
            event_object.object.position.x,
            event_object.object.position.y,
            event_object.object.position.z,
        ) + object_offset,
    }
}

/// Sampler counters proving which obstacle sources actually contribute.
/// Logged per zone at startup so the blocker rule stays observable.
#[derive(Default)]
struct NavBakeStats {
    ifo_blocks: usize,
    collision_obbs: usize,
    cnst_instances: usize,
    deco_instances: usize,
    parts_with_shape: usize,
    parts_height_only: usize,
    parts_passthrough: usize,
    parts_blocking: usize,
    zms_cache_hits: usize,
    zms_reads: usize,
    zms_failures: usize,
    fallback_radii: usize,
    obb_scale_sum: f64,
    obb_scale_min: f32,
    obb_scale_max: f32,
}

/// Unscaled horizontal mesh radius in cm, cached globally across zones.
fn zms_base_radius(
    vfs: &VirtualFilesystem,
    mesh_path: &VfsPathBuf,
    cache: &mut HashMap<String, Option<f32>>,
    stats: &mut NavBakeStats,
) -> Option<f32> {
    // Borrowed for the lookup: most calls are cache hits and need no owned key.
    let key = mesh_path.path().to_string_lossy();
    if let Some(cached) = cache.get(&*key) {
        stats.zms_cache_hits += 1;
        return *cached;
    }
    let key = key.into_owned();
    stats.zms_reads += 1;
    // Cap unique mesh reads so a corrupt zone cannot stall startup.
    if stats.zms_reads > 4000 {
        cache.insert(key, None);
        return None;
    }
    let radius = vfs
        .open_file(VfsPath::from(mesh_path))
        .ok()
        .and_then(|file| match file {
            VfsFile::Buffer(bytes) => ZmsFile::read_bounds(&bytes),
            VfsFile::View(bytes) => ZmsFile::read_bounds(bytes),
        })
        .map(|(bb_min, bb_max)| {
            ((bb_max[0] - bb_min[0]).max(bb_max[1] - bb_min[1]).max(0.0)) * 0.5
        })
        .filter(|r| r.is_finite() && *r > 0.0);
    if radius.is_none() {
        stats.zms_failures += 1;
    }
    cache.insert(key, radius);
    radius
}

/// Footprint radius (cm, unscaled-center) for one cnst/deco instance.
/// Mirrors the client's wall rule in
/// `rose-offline-client/src/zone_loader/spawning/objects.rs`: parts with a
/// collision shape that are neither `HEIGHT_ONLY` (walkable steps) nor
/// `PASSTHROUGH` count as walls. Returns `None` when nothing blocks.
#[allow(clippy::too_many_arguments)]
fn blocker_radius(
    zsc: &ZscFile,
    object_id: usize,
    obj_scale_x: f32,
    obj_scale_y: f32,
    vfs: &VirtualFilesystem,
    mesh_cache: &mut HashMap<String, Option<f32>>,
    stats: &mut NavBakeStats,
) -> Option<f32> {
    let zsc_object = zsc.objects.get(object_id)?;
    let mut radius = 0.0f32;
    let mut blocking = false;
    for part in zsc_object.parts.iter() {
        if part.collision_shape.is_none() {
            continue;
        }
        stats.parts_with_shape += 1;
        if part.collision_flags.contains(ZscCollisionFlags::HEIGHT_ONLY) {
            stats.parts_height_only += 1;
            continue;
        }
        if part.collision_flags.contains(ZscCollisionFlags::PASSTHROUGH) {
            stats.parts_passthrough += 1;
            continue;
        }
        stats.parts_blocking += 1;
        blocking = true;

        let part_scale = part.scale.x.max(part.scale.y).max(0.01);
        let obj_scale = obj_scale_x.max(obj_scale_y).max(0.01);
        let offset_cm = (part.position.x * part.position.x + part.position.y * part.position.y)
            .sqrt()
            * obj_scale;

        if let Some(mesh_path) = zsc.meshes.get(part.mesh_id as usize) {
            if let Some(base) = zms_base_radius(vfs, mesh_path, mesh_cache, stats) {
                radius = radius.max(offset_cm + base * part_scale * obj_scale);
                continue;
            }
        }
        // Mesh or bounds unavailable: fall back to part spread + door width.
        stats.fallback_radii += 1;
        radius = radius.max(offset_cm + 200.0 * part_scale * obj_scale);
    }
    if blocking {
        Some(radius.max(100.0))
    } else {
        None
    }
}

fn load_zone(
    vfs: &VirtualFilesystem,
    data: &StbZone,
    string_database: &StringDatabase,
    id: usize,
    mesh_radius_cache: &mut HashMap<String, Option<f32>>,
    zsc_cache: &mut HashMap<String, Option<ZscFile>>,
) -> Result<ZoneData, LoadZoneError> {
    let zone_file = VfsPath::from(data.get_zone_file(id).ok_or(LoadZoneError::NotExists)?);
    let zone_base_directory = zone_file
        .path()
        .parent()
        .ok_or(LoadZoneError::ZonFileInvalidPath)?;

    let zon_file: ZonFile = vfs
        .read_file_with(
            &zone_file,
            &ZonReadOptions {
                skip_zone_info: false,
                skip_event_positions: false,
                skip_textures: true,
                skip_tiles: true,
            },
        )
        .map_err(|_| LoadZoneError::ZonFileNotFound)?;

    let mut monster_spawns = Vec::new();
    let mut npcs = Vec::new();
    let mut event_objects = Vec::new();

    let mut num_blocks = 0;
    let mut min_block_x = None;
    let mut min_block_y = None;
    let mut max_block_x = None;
    let mut max_block_y = None;

    let objects_offset = Vec3::new(
        (64.0 / 2.0) * (zon_file.grid_size * zon_file.grid_per_patch * 16.0)
            + (zon_file.grid_size * zon_file.grid_per_patch * 16.0) / 2.0,
        (64.0 / 2.0) * (zon_file.grid_size * zon_file.grid_per_patch * 16.0)
            + (zon_file.grid_size * zon_file.grid_per_patch * 16.0) / 2.0,
        0.0,
    );

    let ifo_read_options = IfoReadOptions {
        skip_event_objects: false,
        skip_monster_spawns: false,
        skip_npcs: false,
        skip_animated_objects: true,
        // Loaded (not skipped) for server-side navigation baking: invisible
        // walls (collision) + buildings/props (cnst/deco) are what bots must
        // route around. Matches client collider sources in
        // `rose-offline-client/src/zone_loader/spawning.rs`.
        skip_collision_objects: false,
        skip_cnst_objects: false,
        skip_deco_objects: false,
        skip_effect_objects: true,
        skip_sound_objects: true,
        skip_water_planes: true,
        skip_warp_objects: true,
    };

    // ZSC tables resolve cnst/deco object ids to collision shapes. Missing
    // tables only disable prop blocking; invisible walls still apply.
    // Zones share tables, so each table path is read once per database load.
    let cnst_table = data.get_zone_cnst_table(id).filter(|p| !p.is_empty());
    let deco_table = data.get_zone_deco_table(id).filter(|p| !p.is_empty());
    for table in cnst_table.into_iter().chain(deco_table) {
        if !zsc_cache.contains_key(table) {
            let zsc = vfs.read_file::<ZscFile, _>(&VfsPath::from(table)).ok();
            zsc_cache.insert(table.to_string(), zsc);
        }
    }
    let zsc_cnst: Option<&ZscFile> = cnst_table.and_then(|p| zsc_cache.get(p)?.as_ref());
    let zsc_deco: Option<&ZscFile> = deco_table.and_then(|p| zsc_cache.get(p)?.as_ref());
    if zsc_cnst.is_none() {
        warn!("Nav bake zone {}: missing CNST table, skipping prop blockers", id);
    }
    if zsc_deco.is_none() {
        warn!("Nav bake zone {}: missing DECO table, skipping prop blockers", id);
    }

    let mut nav_stats = NavBakeStats {
        obb_scale_min: f32::MAX,
        ..Default::default()
    };
    // (world_x, world_y, radius_cm)
    let mut nav_circles: Vec<(f32, f32, f32)> = Vec::new();
    // (world_x, world_y, half_x_cm, half_y_cm, yaw_rad)
    let mut nav_obbs: Vec<(f32, f32, f32, f32, f32)> = Vec::new();

    // Most of the 64x64 block names do not exist. Skip the ones no device can
    // open (same blocks in the same order); a device that cannot list the
    // directory makes every name probed as before.
    let block_files = vfs.list_dir(zone_base_directory);
    for block_y in 0..64u32 {
        for block_x in 0..64u32 {
            let ifo_path =
                VfsPath::from(zone_base_directory.join(format!("{}_{}.IFO", block_x, block_y)));
            if !block_files.may_exist(&ifo_path) {
                continue;
            }
            if let Ok(ifo_file) = vfs.read_file_with::<IfoFile, _>(&ifo_path, &ifo_read_options) {
                monster_spawns.extend(
                    ifo_file
                        .monster_spawns
                        .iter()
                        .map(|x| create_monster_spawn(x, objects_offset)),
                );
                npcs.extend(
                    ifo_file
                        .npcs
                        .iter()
                        .filter_map(|x| create_npc_spawn(x, objects_offset)),
                );
                event_objects.extend(ifo_file.event_objects.iter().map(|event_object| {
                    create_event_object(
                        event_object,
                        objects_offset,
                        block_x as i32,
                        block_y as i32,
                    )
                }));

                // --- Nav bake: collect blockers in world (offset) coordinates.
                nav_stats.ifo_blocks += 1;
                for collision in ifo_file.collision_objects.iter() {
                    // Invisible walls: file-frame up is +Z (client remaps quat
                    // (x,y,z,w) -> (x,z,-y,w)), so yaw = 2*atan2(z, w).
                    let yaw = 2.0 * collision.rotation.z.atan2(collision.rotation.w);
                    let sx = collision.scale.x.abs();
                    let sy = collision.scale.y.abs();
                    nav_stats.collision_obbs += 1;
                    nav_stats.obb_scale_sum += sx.max(sy) as f64;
                    nav_stats.obb_scale_min = nav_stats.obb_scale_min.min(sx.max(sy));
                    nav_stats.obb_scale_max = nav_stats.obb_scale_max.max(sx.max(sy));
                    // Scales are box extents in cm; floor at door width so
                    // unit-scale entries still block.
                    nav_obbs.push((
                        collision.position.x + objects_offset.x,
                        collision.position.y + objects_offset.y,
                        (sx * 0.5).max(150.0),
                        (sy * 0.5).max(150.0),
                        yaw,
                    ));
                }
                if let Some(zsc) = zsc_cnst {
                    for instance in ifo_file.cnst_objects.iter() {
                        nav_stats.cnst_instances += 1;
                        if let Some(radius) = blocker_radius(
                            zsc,
                            instance.object_id as usize,
                            instance.scale.x,
                            instance.scale.y,
                            vfs,
                            &mut *mesh_radius_cache,
                            &mut nav_stats,
                        ) {
                            nav_circles.push((
                                instance.position.x + objects_offset.x,
                                instance.position.y + objects_offset.y,
                                radius,
                            ));
                        }
                    }
                }
                if let Some(zsc) = zsc_deco {
                    for instance in ifo_file.deco_objects.iter() {
                        nav_stats.deco_instances += 1;
                        if let Some(radius) = blocker_radius(
                            zsc,
                            instance.object_id as usize,
                            instance.scale.x,
                            instance.scale.y,
                            vfs,
                            &mut *mesh_radius_cache,
                            &mut nav_stats,
                        ) {
                            nav_circles.push((
                                instance.position.x + objects_offset.x,
                                instance.position.y + objects_offset.y,
                                radius,
                            ));
                        }
                    }
                }

                num_blocks += 1;

                min_block_x = Some(min_block_x.map_or(block_x, |value| u32::min(value, block_x)));
                min_block_y = Some(min_block_y.map_or(block_y, |value| u32::min(value, block_y)));
                max_block_x = Some(max_block_x.map_or(block_x, |value| u32::max(value, block_x)));
                max_block_y = Some(max_block_y.map_or(block_y, |value| u32::max(value, block_y)));
            }
        }
    }

    if min_block_x.is_none()
        || min_block_y.is_none()
        || max_block_x.is_none()
        || max_block_y.is_none()
    {
        return Err(LoadZoneError::NotExists);
    }

    let min_x = min_block_x.unwrap();
    let min_y = min_block_y.unwrap() + 1;
    let max_x = max_block_x.unwrap() + 1;
    let max_y = max_block_y.unwrap() + 2;

    let sector_size = data
        .get_zone_sector_size(id)
        .unwrap_or(0)
        .clamp(MIN_SECTOR_SIZE, MAX_SECTOR_SIZE);
    let block_size = 16.0 * zon_file.grid_per_patch * zon_file.grid_size;
    let num_blocks_x = max_x - min_x;
    let num_blocks_y = max_y - min_y;
    let num_sectors_x = ((num_blocks_x as f32 * block_size) / sector_size as f32) as u32;
    let num_sectors_y = ((num_blocks_y as f32 * block_size) / sector_size as f32) as u32;

    let start_event_position_name = data.get_zone_start_event_position_name(id).unwrap_or("");
    let revive_event_position_name = data.get_zone_revive_event_position_name(id).unwrap_or("");
    let mut start_position = Vec3::new(0.0, 0.0, 0.0);
    let mut revive_positions = Vec::new();
    for (name, position) in zon_file.event_positions.iter() {
        let position = Vec3::new(position.x, position.y, position.z).xzy() + objects_offset;

        if name == start_event_position_name {
            start_position = position;
        }

        if name == revive_event_position_name {
            revive_positions.push(position);
        }
    }

    let zone_strings = string_database.get_zone(data.get_zone_string_id(id).unwrap_or(""));
    let name = zone_strings
        .as_ref()
        .map_or(String::new(), |x| x.name.to_string());
    let description = zone_strings
        .as_ref()
        .map_or(String::new(), |x| x.description.to_string());
    debug!(
        "Loaded zone {} {} blocks: {}, spawns: {}, npcs: {}, sectors ({}, {}), start: {}",
        id,
        name,
        num_blocks,
        monster_spawns.len(),
        npcs.len(),
        num_sectors_x,
        num_sectors_y,
        start_position.xy(),
    );
    // --- Nav bake: rasterize blockers + anchor bounds on real content.
    // Bounds cover every blocker footprint plus every walk goal (spawns,
    // npcs, start) so paths between any two goals stay inside the grid.
    let mut nav_grid = None;
    if !nav_circles.is_empty() || !nav_obbs.is_empty() {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        let mut grow = |x: f32, y: f32, r: f32| {
            min_x = min_x.min(x - r);
            min_y = min_y.min(y - r);
            max_x = max_x.max(x + r);
            max_y = max_y.max(y + r);
        };
        for (x, y, r) in nav_circles.iter() {
            grow(*x, *y, *r);
        }
        for (x, y, hx, hy, _) in nav_obbs.iter() {
            grow(*x, *y, hx.max(*hy));
        }
        for spawn in monster_spawns.iter() {
            grow(spawn.position.x, spawn.position.y, 0.0);
        }
        for npc in npcs.iter() {
            grow(npc.position.x, npc.position.y, 0.0);
        }
        grow(start_position.x, start_position.y, 0.0);

        if min_x < max_x && min_y < max_y {
            const NAV_MARGIN_CM: f32 = 800.0;
            let mut grid = NavGrid::covering(
                min_x - NAV_MARGIN_CM,
                min_y - NAV_MARGIN_CM,
                max_x + NAV_MARGIN_CM,
                max_y + NAV_MARGIN_CM,
            );
            for (x, y, r) in nav_circles.iter() {
                grid.mark_circle(*x, *y, *r + NAV_AGENT_RADIUS_CM);
            }
            for (x, y, hx, hy, yaw) in nav_obbs.iter() {
                grid.mark_obb(
                    *x,
                    *y,
                    *hx + NAV_AGENT_RADIUS_CM,
                    *hy + NAV_AGENT_RADIUS_CM,
                    *yaw,
                );
            }
            debug!(
                "Nav bake zone {}: {}x{} cells @ {:.0}cm, blocked {}/{} cells, obbs={}, circles={} (cnst={}, deco={}), parts shape={} blocking={} (height_only={}, passthrough={}), zms reads={} hits={} fail={} fallback={}",
                id,
                grid.width,
                grid.height,
                grid.cell_cm,
                grid.blocked_cell_count(),
                grid.blocked.len(),
                nav_stats.collision_obbs,
                nav_circles.len(),
                nav_stats.cnst_instances,
                nav_stats.deco_instances,
                nav_stats.parts_with_shape,
                nav_stats.parts_blocking,
                nav_stats.parts_height_only,
                nav_stats.parts_passthrough,
                nav_stats.zms_reads,
                nav_stats.zms_cache_hits,
                nav_stats.zms_failures,
                nav_stats.fallback_radii,
            );
            if nav_stats.collision_obbs > 0 {
                debug!(
                    "Nav bake zone {}: obb scale avg={:.1} min={:.1} max={:.1}",
                    id,
                    nav_stats.obb_scale_sum / nav_stats.collision_obbs as f64,
                    nav_stats.obb_scale_min,
                    nav_stats.obb_scale_max,
                );
            }
            nav_grid = Some(grid);
        }
    } else {
        debug!(
            "Nav bake zone {}: no blockers found (obbs=0 circles=0), grid skipped",
            id
        );
    }

    Ok(ZoneData {
        id: ZoneId::new(id as u16).ok_or(LoadZoneError::NotExists)?,
        name,
        description,
        sector_size,
        grid_per_patch: zon_file.grid_per_patch,
        grid_size: zon_file.grid_size,
        event_objects,
        monster_spawns,
        npcs,
        sectors_base_position: Vec2::new((min_x as f32) * block_size, (min_y as f32) * block_size),
        num_sectors_x,
        num_sectors_y,
        start_position,
        revive_positions,
        event_positions: zon_file
            .event_positions
            .into_iter()
            .map(|(name, position)| {
                (
                    name,
                    Vec3::new(position.x, position.y, position.z).xzy() + objects_offset,
                )
            })
            .collect(),
        day_cycle: data
            .get_zone_day_cycle_time(id)
            .unwrap_or(WORLD_TICKS_PER_DAY as u32),
        morning_time: data
            .get_zone_morning_time(id)
            .unwrap_or((WORLD_TICKS_PER_DAY / 6) as u32),
        day_time: data
            .get_zone_day_time(id)
            .unwrap_or((2 * WORLD_TICKS_PER_DAY / 6) as u32),
        evening_time: data
            .get_zone_evening_time(id)
            .unwrap_or((4 * WORLD_TICKS_PER_DAY / 6) as u32),
        night_time: data
            .get_zone_night_time(id)
            .unwrap_or((5 * WORLD_TICKS_PER_DAY / 6) as u32),
        skybox_id: data.get_zone_skybox_id(id),
        planet: data.get_zone_planet(id).unwrap_or(0) as usize,
        nav_grid,
    })
}

fn ensure_zone_slot<T>(zones: &mut Vec<Option<T>>, id: usize) {
    if zones.len() <= id {
        zones.resize_with(id + 1, || None);
    }
}

fn load_synthetic_ocean_zone(vfs: &VirtualFilesystem) -> Result<ZoneData, LoadZoneError> {
    let zone_file = VfsPath::from(OCEAN_ZONE_FILE);
    let zone_base_directory = zone_file
        .path()
        .parent()
        .ok_or(LoadZoneError::ZonFileInvalidPath)?;

    let zon_file: ZonFile = vfs
        .read_file_with(
            &zone_file,
            &ZonReadOptions {
                skip_zone_info: false,
                skip_event_positions: false,
                skip_textures: true,
                skip_tiles: true,
            },
        )
        .map_err(|_| LoadZoneError::ZonFileNotFound)?;

    let mut monster_spawns = Vec::new();
    let mut npcs = Vec::new();
    let mut event_objects = Vec::new();

    let mut num_blocks = 0;
    let mut min_block_x = None;
    let mut min_block_y = None;
    let mut max_block_x = None;
    let mut max_block_y = None;

    let objects_offset = Vec3::new(
        (64.0 / 2.0) * (zon_file.grid_size * zon_file.grid_per_patch * 16.0)
            + (zon_file.grid_size * zon_file.grid_per_patch * 16.0) / 2.0,
        (64.0 / 2.0) * (zon_file.grid_size * zon_file.grid_per_patch * 16.0)
            + (zon_file.grid_size * zon_file.grid_per_patch * 16.0) / 2.0,
        0.0,
    );

    let ifo_read_options = IfoReadOptions {
        skip_event_objects: false,
        skip_monster_spawns: false,
        skip_npcs: false,
        skip_animated_objects: true,
        skip_collision_objects: true,
        skip_cnst_objects: true,
        skip_deco_objects: true,
        skip_effect_objects: true,
        skip_sound_objects: true,
        skip_water_planes: true,
        skip_warp_objects: true,
    };

    // Skip block names no device can open, as `load_zone` does.
    let block_files = vfs.list_dir(zone_base_directory);
    for block_y in 0..64u32 {
        for block_x in 0..64u32 {
            let ifo_path =
                VfsPath::from(zone_base_directory.join(format!("{}_{}.IFO", block_x, block_y)));
            if !block_files.may_exist(&ifo_path) {
                continue;
            }
            if let Ok(ifo_file) = vfs.read_file_with::<IfoFile, _>(&ifo_path, &ifo_read_options) {
                monster_spawns.extend(
                    ifo_file
                        .monster_spawns
                        .iter()
                        .map(|x| create_monster_spawn(x, objects_offset)),
                );
                npcs.extend(
                    ifo_file
                        .npcs
                        .iter()
                        .filter_map(|x| create_npc_spawn(x, objects_offset)),
                );
                event_objects.extend(ifo_file.event_objects.iter().map(|event_object| {
                    create_event_object(
                        event_object,
                        objects_offset,
                        block_x as i32,
                        block_y as i32,
                    )
                }));
                num_blocks += 1;

                min_block_x = Some(min_block_x.map_or(block_x, |value| u32::min(value, block_x)));
                min_block_y = Some(min_block_y.map_or(block_y, |value| u32::min(value, block_y)));
                max_block_x = Some(max_block_x.map_or(block_x, |value| u32::max(value, block_x)));
                max_block_y = Some(max_block_y.map_or(block_y, |value| u32::max(value, block_y)));
            }
        }
    }

    if min_block_x.is_none()
        || min_block_y.is_none()
        || max_block_x.is_none()
        || max_block_y.is_none()
    {
        return Err(LoadZoneError::NotExists);
    }

    let min_x = min_block_x.unwrap();
    let min_y = min_block_y.unwrap() + 1;
    let max_x = max_block_x.unwrap() + 1;
    let max_y = max_block_y.unwrap() + 2;

    let sector_size = OCEAN_SECTOR_SIZE.clamp(MIN_SECTOR_SIZE, MAX_SECTOR_SIZE);
    let block_size = 16.0 * zon_file.grid_per_patch * zon_file.grid_size;
    let num_blocks_x = max_x - min_x;
    let num_blocks_y = max_y - min_y;
    let num_sectors_x = ((num_blocks_x as f32 * block_size) / sector_size as f32) as u32;
    let num_sectors_y = ((num_blocks_y as f32 * block_size) / sector_size as f32) as u32;

    let default_position = Vec3::new(objects_offset.x, objects_offset.y, 180.0);
    let mut start_position = default_position;
    let mut revive_positions = Vec::new();
    for (name, position) in zon_file.event_positions.iter() {
        let position = Vec3::new(position.x, position.y, position.z).xzy() + objects_offset;

        if name == OCEAN_START_EVENT {
            start_position = position;
        }

        if name == OCEAN_REVIVE_EVENT {
            revive_positions.push(position);
        }
    }

    if revive_positions.is_empty() {
        revive_positions.push(start_position);
    }

    debug!(
        "Loaded synthetic ocean zone {} blocks: {}, sectors ({}, {}), start: {}",
        OCEAN_ZONE_ID,
        num_blocks,
        num_sectors_x,
        num_sectors_y,
        start_position.xy(),
    );

    Ok(ZoneData {
        id: ZoneId::new(OCEAN_ZONE_ID_U16).ok_or(LoadZoneError::NotExists)?,
        name: OCEAN_ZONE_NAME.to_string(),
        description: OCEAN_ZONE_DESCRIPTION.to_string(),
        sector_size,
        grid_per_patch: zon_file.grid_per_patch,
        grid_size: zon_file.grid_size,
        event_objects,
        monster_spawns,
        npcs,
        sectors_base_position: Vec2::new((min_x as f32) * block_size, (min_y as f32) * block_size),
        num_sectors_x,
        num_sectors_y,
        start_position,
        revive_positions,
        event_positions: zon_file
            .event_positions
            .into_iter()
            .map(|(name, position)| {
                (
                    name,
                    Vec3::new(position.x, position.y, position.z).xzy() + objects_offset,
                )
            })
            .collect(),
        day_cycle: WORLD_TICKS_PER_DAY as u32,
        morning_time: (WORLD_TICKS_PER_DAY / 6) as u32,
        day_time: (2 * WORLD_TICKS_PER_DAY / 6) as u32,
        evening_time: (4 * WORLD_TICKS_PER_DAY / 6) as u32,
        night_time: (5 * WORLD_TICKS_PER_DAY / 6) as u32,
        skybox_id: None,
        planet: OCEAN_PLANET,
        // Open water: boats steer with their own terrain-height checks.
        nav_grid: None,
    })
}

pub fn get_zone_database(
    vfs: &VirtualFilesystem,
    string_database: Arc<StringDatabase>,
) -> Result<ZoneDatabase, anyhow::Error> {
    let data = StbZone(vfs.read_file::<StbFile, _>("3DDATA/STB/LIST_ZONE.STB")?);
    let mut zones = Vec::with_capacity(data.rows());
    zones.push(None); // Zone ID 0
    // Shared across zones: most prop meshes repeat everywhere, so later zones
    // bake almost entirely from cache.
    let mut mesh_radius_cache: HashMap<String, Option<f32>> = HashMap::new();
    let mut zsc_cache: HashMap<String, Option<ZscFile>> = HashMap::new();
    for id in 1..data.rows() {
        zones.push(
            load_zone(
                vfs,
                &data,
                &string_database,
                id,
                &mut mesh_radius_cache,
                &mut zsc_cache,
            )
            .ok(),
        );
    }

    let needs_ocean_zone = zones.get(OCEAN_ZONE_ID).map_or(true, Option::is_none);
    if needs_ocean_zone && vfs.exists(OCEAN_ZONE_FILE) {
        ensure_zone_slot(&mut zones, OCEAN_ZONE_ID);
        zones[OCEAN_ZONE_ID] = load_synthetic_ocean_zone(vfs).ok();
    }

    Ok(ZoneDatabase::new(string_database, zones))
}

fn load_zone_list_entry(
    data: &StbZone,
    string_database: &StringDatabase,
    id: usize,
) -> Result<ZoneListEntry, LoadZoneError> {
    let zone_strings = string_database.get_zone(data.get_zone_string_id(id).unwrap_or(""));

    Ok(ZoneListEntry {
        id: ZoneId::new(id as u16).ok_or(LoadZoneError::NotExists)?,
        name: zone_strings
            .as_ref()
            .map_or(String::new(), |x| x.name.to_string()),
        description: zone_strings
            .as_ref()
            .map_or(String::new(), |x| x.description.to_string()),
        minimap_path: data.get_zone_minimap_filename(id).map(VfsPathBuf::new),
        minimap_start_x: data.get_zone_minimap_start_x(id).unwrap_or(0),
        minimap_start_y: data.get_zone_minimap_start_y(id).unwrap_or(0),
        zon_file_path: VfsPathBuf::new(data.get_zone_file(id).ok_or(LoadZoneError::NotExists)?),
        zsc_cnst_path: VfsPathBuf::new(data.get_zone_cnst_table(id).unwrap_or("")),
        zsc_deco_path: VfsPathBuf::new(data.get_zone_deco_table(id).unwrap_or("")),
        day_cycle: data
            .get_zone_day_cycle_time(id)
            .unwrap_or(WORLD_TICKS_PER_DAY as u32),
        morning_time: data
            .get_zone_morning_time(id)
            .unwrap_or((WORLD_TICKS_PER_DAY / 6) as u32),
        day_time: data
            .get_zone_day_time(id)
            .unwrap_or((2 * WORLD_TICKS_PER_DAY / 6) as u32),
        evening_time: data
            .get_zone_evening_time(id)
            .unwrap_or((4 * WORLD_TICKS_PER_DAY / 6) as u32),
        night_time: data
            .get_zone_night_time(id)
            .unwrap_or((5 * WORLD_TICKS_PER_DAY / 6) as u32),
        skybox_id: data.get_zone_skybox_id(id),
        background_music_day: data.get_zone_background_music_day(id).map(VfsPathBuf::new),
        background_music_night: data
            .get_zone_background_music_night(id)
            .map(VfsPathBuf::new),
        footstep_type: data.get_zone_footstep_type(id),
    })
}

fn load_synthetic_ocean_zone_list_entry() -> Result<ZoneListEntry, LoadZoneError> {
    Ok(ZoneListEntry {
        id: ZoneId::new(OCEAN_ZONE_ID_U16).ok_or(LoadZoneError::NotExists)?,
        name: OCEAN_ZONE_NAME.to_string(),
        description: OCEAN_ZONE_DESCRIPTION.to_string(),
        minimap_path: None,
        minimap_start_x: 0,
        minimap_start_y: 0,
        zon_file_path: VfsPathBuf::new(OCEAN_ZONE_FILE),
        zsc_cnst_path: VfsPathBuf::new(OCEAN_CNST_TABLE),
        zsc_deco_path: VfsPathBuf::new(OCEAN_DECO_TABLE),
        day_cycle: WORLD_TICKS_PER_DAY as u32,
        morning_time: (WORLD_TICKS_PER_DAY / 6) as u32,
        day_time: (2 * WORLD_TICKS_PER_DAY / 6) as u32,
        evening_time: (4 * WORLD_TICKS_PER_DAY / 6) as u32,
        night_time: (5 * WORLD_TICKS_PER_DAY / 6) as u32,
        skybox_id: None,
        background_music_day: None,
        background_music_night: None,
        footstep_type: Some(0),
    })
}

pub fn get_zone_list(
    vfs: &VirtualFilesystem,
    string_database: Arc<StringDatabase>,
) -> Result<ZoneList, anyhow::Error> {
    let data = StbZone(vfs.read_file::<StbFile, _>("3DDATA/STB/LIST_ZONE.STB")?);
    let mut zones = Vec::with_capacity(data.rows());
    zones.push(None); // Zone ID 0
    for id in 1..data.rows() {
        zones.push(load_zone_list_entry(&data, &string_database, id).ok());
    }

    let needs_ocean_zone = zones.get(OCEAN_ZONE_ID).map_or(true, Option::is_none);
    if needs_ocean_zone && vfs.exists(OCEAN_ZONE_FILE) {
        ensure_zone_slot(&mut zones, OCEAN_ZONE_ID);
        zones[OCEAN_ZONE_ID] = load_synthetic_ocean_zone_list_entry().ok();
    }

    Ok(ZoneList::new(string_database, zones))
}
