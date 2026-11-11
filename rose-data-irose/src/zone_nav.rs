//! Collects a zone's walkability bake input ([`NavBakeInput`]) from the game
//! files: HIM terrain, IFO placements expanded into the ZMS triangles of
//! their ZSC collision parts, water planes and warp gates. The bake itself is
//! `rose_data::bake_nav_grid`.
//!
//! Placement and collision rules mirror the client
//! (`rose-offline-client/src/zone_loader/spawning/objects.rs`):
//! - world = IFO position + zone object offset (cm, Z-up), IFO quaternion and
//!   per-axis scale; ZSC part position (cm), quaternion and scale relative to
//!   the object (part parent links are not followed, as in the client);
//!   ZMS vertices are meters.
//! - a part takes part in movement only with a collision shape; grass-kind
//!   meshes (grass, bushes, plants) are walk-through.
//! - collidable (blocks) unless `HEIGHT_ONLY`; moveable (stood on) unless
//!   `NOT_MOVEABLE`. Event objects are only stood on. The IFO collision lump
//!   places the designers' invisible wall panel (special object 2).
//! - HIM row 0 is the block's north edge; block `(x, y)` spans
//!   x = `x * size ..`, y = `(64 - y) * size .. (65 - y) * size`.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::math::{Affine3A, Quat, Vec2, Vec3};
use log::warn;

use rose_data::{NavBakeInput, NavBakeTriangle, NavTerrainBlock, NavWaterPlane};
use rose_file_readers::{
    HimFile, IfoFile, IfoObject, VfsPath, VfsPathBuf, VirtualFilesystem, ZmsFile,
    ZscCollisionFlags, ZscFile,
};

const SPECIAL_OBJECT_TABLE: &str = "3DDATA/SPECIAL/LIST_DECO_SPECIAL.ZSC";
const EVENT_OBJECT_TABLE: &str = "3DDATA/SPECIAL/EVENT_OBJECT.ZSC";
/// Special table object of IFO warp gates (as the client spawns them).
const SPECIAL_WARP_OBJECT: usize = 1;
/// Special table object of IFO collision lump records: `Collision01.zms`.
const SPECIAL_COLLISION_OBJECT: usize = 2;
/// Keep-out margin around a warp gate's footprint: walking in changes zone.
const WARP_KEEP_OUT_MARGIN_CM: f32 = 150.0;
/// Keep-out half size when the warp gate mesh is unavailable.
const WARP_FALLBACK_HALF_CM: f32 = 400.0;
/// Cap on unique mesh reads per database load, so a corrupt install cannot
/// stall startup.
const MAX_MESH_READS: usize = 30000;

type MeshTriangles = Arc<Vec<[Vec3; 3]>>;

/// How a placed object's collision parts take part in movement.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PartRule {
    /// cnst/deco objects: the client's collidable / moveable rule.
    Zone,
    /// Event objects: stood on, never blocking.
    FloorOnly,
    /// Collision lump records: invisible walls.
    WallOnly,
}

/// Meshes and special tables shared by every zone of one database load.
pub struct NavAssets {
    meshes: HashMap<String, Option<MeshTriangles>>,
    special: Option<ZscFile>,
    event: Option<ZscFile>,
    mesh_reads: usize,
    mesh_failures: usize,
}

impl NavAssets {
    pub fn new(vfs: &VirtualFilesystem) -> Self {
        let special = vfs.read_file::<ZscFile, _>(SPECIAL_OBJECT_TABLE).ok();
        if special.is_none() {
            warn!(
                "Nav bake: {} missing, invisible walls and warp gates use fallbacks",
                SPECIAL_OBJECT_TABLE
            );
        }
        Self {
            meshes: HashMap::new(),
            special,
            event: vfs.read_file::<ZscFile, _>(EVENT_OBJECT_TABLE).ok(),
            mesh_reads: 0,
            mesh_failures: 0,
        }
    }

    pub fn mesh_reads(&self) -> usize {
        self.mesh_reads
    }

    pub fn mesh_failures(&self) -> usize {
        self.mesh_failures
    }

    /// Mesh triangles in object-local cm, cached across zones.
    fn mesh_triangles(&mut self, vfs: &VirtualFilesystem, path: &VfsPathBuf) -> Option<MeshTriangles> {
        let key = path.path().to_string_lossy().to_ascii_lowercase();
        if let Some(cached) = self.meshes.get(&key) {
            return cached.clone();
        }
        let triangles = if self.mesh_reads < MAX_MESH_READS {
            self.mesh_reads += 1;
            vfs.read_file::<ZmsFile, _>(VfsPath::from(path))
                .ok()
                .map(|zms| {
                    let vertex = |index: u32| {
                        zms.position
                            .get(index as usize)
                            .map(|[x, y, z]| Vec3::new(*x, *y, *z) * 100.0)
                    };
                    Arc::new(
                        zms.indices
                            .chunks_exact(3)
                            .filter_map(|tri| Some([vertex(tri[0])?, vertex(tri[1])?, vertex(tri[2])?]))
                            .collect::<Vec<_>>(),
                    )
                })
        } else {
            None
        };
        if triangles.is_none() {
            self.mesh_failures += 1;
        }
        self.meshes.insert(key, triangles.clone());
        triangles
    }
}

/// Grass-kind meshes are walk-through in the client (`WindSway::for_mesh_path`
/// grass sway): grass, and bushes / shrubs / plants that are not foliage.
fn is_walk_through_mesh(path: &str) -> bool {
    let path = path.to_lowercase();
    if path.contains("grass") {
        return true;
    }
    if ["leaf", "leaves", "foliage", "canopy"]
        .iter()
        .any(|word| path.contains(word))
    {
        return false;
    }
    ["bush", "shrub", "plant"].iter().any(|word| path.contains(word))
}

fn quat(x: f32, y: f32, z: f32, w: f32) -> Quat {
    let q = Quat::from_xyzw(x, y, z, w);
    if q.is_finite() && q.length_squared() > 1e-6 {
        q.normalize()
    } else {
        Quat::IDENTITY
    }
}

/// Unset (zero) scale axes count as 1, as the original placement tools did.
fn scale(x: f32, y: f32, z: f32) -> Vec3 {
    let axis = |v: f32| if v == 0.0 || !v.is_finite() { 1.0 } else { v };
    Vec3::new(axis(x), axis(y), axis(z))
}

/// Counters logged per zone so the bake stays observable.
#[derive(Default, Debug)]
pub struct ZoneNavStats {
    pub terrain_blocks: usize,
    pub missing_terrain_blocks: usize,
    pub placements: usize,
    pub parts: usize,
    pub missing_meshes: usize,
    pub collision_walls: usize,
    pub warps: usize,
    pub water_planes: usize,
}

/// Bake input of one zone, filled block by block while the zone loads.
pub struct ZoneNavCollector {
    block_size: f32,
    objects_offset: Vec3,
    input: NavBakeInput,
    block_bounds: Option<(u32, u32, u32, u32)>,
    pub stats: ZoneNavStats,
}

impl ZoneNavCollector {
    pub fn new(block_size: f32, objects_offset: Vec3) -> Self {
        Self {
            block_size,
            objects_offset,
            input: NavBakeInput::default(),
            block_bounds: None,
            stats: ZoneNavStats::default(),
        }
    }

    fn object_transform(&self, object: &IfoObject) -> Affine3A {
        Affine3A::from_scale_rotation_translation(
            scale(object.scale.x, object.scale.y, object.scale.z),
            quat(
                object.rotation.x,
                object.rotation.y,
                object.rotation.z,
                object.rotation.w,
            ),
            Vec3::new(object.position.x, object.position.y, object.position.z)
                + self.objects_offset,
        )
    }

    /// World-space collision triangles of one placed object.
    fn object_triangles(
        &mut self,
        vfs: &VirtualFilesystem,
        assets: &mut NavAssets,
        table: TableRef,
        object_id: usize,
        object: &IfoObject,
        rule: PartRule,
        mut emit: impl FnMut(&mut Self, [Vec3; 3], bool, bool),
    ) {
        let zsc = match table {
            TableRef::Zone(zsc) => zsc,
            TableRef::Special => match assets.special.as_ref() {
                Some(zsc) => zsc,
                None => return,
            },
            TableRef::Event => match assets.event.as_ref() {
                Some(zsc) => zsc,
                None => return,
            },
        };
        let Some(zsc_object) = zsc.objects.get(object_id) else {
            return;
        };
        let object_transform = self.object_transform(object);
        self.stats.placements += 1;

        // Collect what is needed from the table first: `assets` is borrowed
        // mutably for mesh reads below.
        let parts: Vec<(VfsPathBuf, Affine3A, bool, bool)> = zsc_object
            .parts
            .iter()
            .filter_map(|part| {
                let mesh_path = zsc.meshes.get(part.mesh_id as usize)?;
                let flags = part.collision_flags;
                let (collidable, moveable) = match rule {
                    PartRule::WallOnly => (true, false),
                    PartRule::Zone | PartRule::FloorOnly => {
                        part.collision_shape?;
                        if is_walk_through_mesh(&mesh_path.path().to_string_lossy()) {
                            return None;
                        }
                        (
                            rule == PartRule::Zone
                                && !flags.contains(ZscCollisionFlags::HEIGHT_ONLY),
                            !flags.contains(ZscCollisionFlags::NOT_MOVEABLE),
                        )
                    }
                };
                if !collidable && !moveable {
                    return None;
                }
                let part_transform = Affine3A::from_scale_rotation_translation(
                    scale(part.scale.x, part.scale.y, part.scale.z),
                    quat(part.rotation.x, part.rotation.y, part.rotation.z, part.rotation.w),
                    Vec3::new(part.position.x, part.position.y, part.position.z),
                );
                Some((
                    mesh_path.clone(),
                    object_transform * part_transform,
                    collidable,
                    moveable,
                ))
            })
            .collect();

        for (mesh_path, transform, collidable, moveable) in parts {
            self.stats.parts += 1;
            let Some(triangles) = assets.mesh_triangles(vfs, &mesh_path) else {
                self.stats.missing_meshes += 1;
                continue;
            };
            for tri in triangles.iter() {
                emit(
                    self,
                    tri.map(|v| transform.transform_point3(v)),
                    collidable,
                    moveable,
                );
            }
        }
    }

    /// Everything one IFO block (and its HIM) contributes.
    #[allow(clippy::too_many_arguments)]
    pub fn add_block(
        &mut self,
        vfs: &VirtualFilesystem,
        assets: &mut NavAssets,
        zone_directory: &std::path::Path,
        block_x: u32,
        block_y: u32,
        ifo: &IfoFile,
        zsc_cnst: Option<&ZscFile>,
        zsc_deco: Option<&ZscFile>,
    ) {
        self.block_bounds = Some(match self.block_bounds {
            None => (block_x, block_y, block_x, block_y),
            Some((x0, y0, x1, y1)) => (
                x0.min(block_x),
                y0.min(block_y),
                x1.max(block_x),
                y1.max(block_y),
            ),
        });

        let him_path =
            VfsPath::from(zone_directory.join(format!("{}_{}.HIM", block_x, block_y)));
        match vfs.read_file::<HimFile, _>(&him_path) {
            Ok(him) if him.width >= 2 && him.height >= 2 => {
                self.stats.terrain_blocks += 1;
                self.input.terrain.push(NavTerrainBlock {
                    west_x: block_x as f32 * self.block_size,
                    north_y: (65 - block_y as i32) as f32 * self.block_size,
                    spacing: self.block_size / (him.width - 1) as f32,
                    width: him.width,
                    height: him.height,
                    heights: him.heights,
                });
            }
            _ => self.stats.missing_terrain_blocks += 1,
        }

        let mut push = |collector: &mut Self, vertices: [Vec3; 3], collidable: bool, moveable: bool| {
            collector.input.triangles.push(NavBakeTriangle {
                vertices,
                collidable,
                moveable,
            });
        };
        for (zsc, objects) in [(zsc_cnst, &ifo.cnst_objects), (zsc_deco, &ifo.deco_objects)] {
            let Some(zsc) = zsc else { continue };
            for object in objects.iter() {
                self.object_triangles(
                    vfs,
                    assets,
                    TableRef::Zone(zsc),
                    object.object_id as usize,
                    object,
                    PartRule::Zone,
                    &mut push,
                );
            }
        }
        for event_object in ifo.event_objects.iter() {
            self.object_triangles(
                vfs,
                assets,
                TableRef::Event,
                event_object.object.object_id as usize,
                &event_object.object,
                PartRule::FloorOnly,
                &mut push,
            );
        }
        for collision in ifo.collision_objects.iter() {
            self.stats.collision_walls += 1;
            self.object_triangles(
                vfs,
                assets,
                TableRef::Special,
                SPECIAL_COLLISION_OBJECT,
                collision,
                PartRule::WallOnly,
                &mut push,
            );
        }

        for warp in ifo.warps.iter() {
            self.stats.warps += 1;
            let mut min = Vec2::splat(f32::INFINITY);
            let mut max = Vec2::splat(f32::NEG_INFINITY);
            self.object_triangles(
                vfs,
                assets,
                TableRef::Special,
                SPECIAL_WARP_OBJECT,
                warp,
                PartRule::WallOnly,
                |_, vertices, _, _| {
                    for v in vertices {
                        min = min.min(v.truncate());
                        max = max.max(v.truncate());
                    }
                },
            );
            let (min, max) = if min.x <= max.x {
                (
                    min - Vec2::splat(WARP_KEEP_OUT_MARGIN_CM),
                    max + Vec2::splat(WARP_KEEP_OUT_MARGIN_CM),
                )
            } else {
                let center = Vec2::new(warp.position.x, warp.position.y)
                    + self.objects_offset.truncate();
                (
                    center - Vec2::splat(WARP_FALLBACK_HALF_CM),
                    center + Vec2::splat(WARP_FALLBACK_HALF_CM),
                )
            };
            self.input.keep_out.push((min, max));
        }

        // Water planes are stored Y-up: (x, height, y) relative to the offset.
        for (start, end) in ifo.water_planes.iter() {
            self.stats.water_planes += 1;
            let offset = self.objects_offset.truncate();
            let a = Vec2::new(start.x, start.z) + offset;
            let b = Vec2::new(end.x, end.z) + offset;
            self.input.water.push(NavWaterPlane {
                min: a.min(b),
                max: a.max(b),
                z: start.y.max(end.y),
            });
        }
    }

    /// The finished input, or `None` when the zone has no terrain.
    pub fn finish(mut self, anchors: Vec<Vec3>) -> Option<(NavBakeInput, ZoneNavStats)> {
        let (x0, y0, x1, y1) = self.block_bounds?;
        if self.input.terrain.is_empty() {
            return None;
        }
        let size = self.block_size;
        self.input.min = Vec2::new(x0 as f32 * size, (64 - y1 as i32) as f32 * size);
        self.input.max = Vec2::new((x1 + 1) as f32 * size, (65 - y0 as i32) as f32 * size);
        self.input.anchors = anchors;
        Some((self.input, self.stats))
    }
}

/// Which ZSC table a placement resolves against.
#[derive(Clone, Copy)]
enum TableRef<'a> {
    Zone(&'a ZscFile),
    Special,
    Event,
}

/// Debug listing of every placed object part whose world bounds come within
/// `radius` cm (horizontally) of `x, y` in a zone: what it is, its collision
/// shape and flags, how the bake treats it and its world height range. Also
/// lists IFO animated (morph) objects, which the bake ignores.
pub fn describe_nav_objects_near(
    vfs: &VirtualFilesystem,
    zone_id: usize,
    x: f32,
    y: f32,
    radius: f32,
) -> anyhow::Result<Vec<String>> {
    use rose_file_readers::{IfoReadOptions, StbFile, ZonFile, ZonReadOptions};

    let stb = crate::zone_database::StbZone(
        vfs.read_file::<StbFile, _>("3DDATA/STB/LIST_ZONE.STB")?,
    );
    let zone_file = VfsPath::from(
        stb.get_zone_file(zone_id)
            .ok_or_else(|| anyhow::anyhow!("zone {} has no ZON file", zone_id))?,
    );
    let zone_directory = zone_file
        .path()
        .parent()
        .ok_or_else(|| anyhow::anyhow!("bad ZON path"))?
        .to_path_buf();
    let zon: ZonFile = vfs.read_file_with(
        &zone_file,
        &ZonReadOptions {
            skip_zone_info: false,
            skip_event_positions: true,
            skip_textures: true,
            skip_tiles: true,
        },
    )?;
    let block_size = 16.0 * zon.grid_per_patch * zon.grid_size;
    let offset = Vec3::new(32.5 * block_size, 32.5 * block_size, 0.0);
    let read_table = |path: Option<&str>| {
        path.filter(|p| !p.is_empty())
            .and_then(|p| vfs.read_file::<ZscFile, _>(VfsPath::from(p)).ok())
    };
    let zsc_cnst = read_table(stb.get_zone_cnst_table(zone_id));
    let zsc_deco = read_table(stb.get_zone_deco_table(zone_id));
    let mut assets = NavAssets::new(vfs);
    // Owned copies, so mesh reads can borrow `assets` mutably.
    let special = assets.special.clone();
    let event = assets.event.clone();
    let collector = ZoneNavCollector::new(block_size, offset);
    let options = IfoReadOptions {
        skip_event_objects: false,
        skip_monster_spawns: true,
        skip_npcs: true,
        skip_animated_objects: false,
        skip_collision_objects: false,
        skip_cnst_objects: false,
        skip_deco_objects: false,
        skip_effect_objects: true,
        skip_sound_objects: true,
        skip_water_planes: false,
        skip_warp_objects: false,
    };

    let center = Vec2::new(x, y);
    let block_x = (x / block_size).floor() as i32;
    let block_y = 64 - (y / block_size).floor() as i32;
    let mut lines = Vec::new();
    for by in block_y - 1..=block_y + 1 {
        for bx in block_x - 1..=block_x + 1 {
            if !(0..64).contains(&bx) || !(0..64).contains(&by) {
                continue;
            }
            let ifo_path = VfsPath::from(zone_directory.join(format!("{}_{}.IFO", bx, by)));
            let Ok(ifo) = vfs.read_file_with::<IfoFile, _>(&ifo_path, &options) else {
                continue;
            };
            for object in ifo.animated_objects.iter() {
                let p = Vec3::new(object.position.x, object.position.y, object.position.z) + offset;
                if p.truncate().distance(center) <= radius {
                    lines.push(format!(
                        "block {}_{} ANIMATED (ignored by bake) obj {} at ({:.0}, {:.0}, {:.0})",
                        bx, by, object.object_id, p.x, p.y, p.z
                    ));
                }
            }
            let mut placements: Vec<(&str, TableRef, usize, &IfoObject)> = Vec::new();
            if let Some(zsc) = zsc_cnst.as_ref() {
                placements.extend(
                    ifo.cnst_objects
                        .iter()
                        .map(|o| ("CNST", TableRef::Zone(zsc), o.object_id as usize, o)),
                );
            }
            if let Some(zsc) = zsc_deco.as_ref() {
                placements.extend(
                    ifo.deco_objects
                        .iter()
                        .map(|o| ("DECO", TableRef::Zone(zsc), o.object_id as usize, o)),
                );
            }
            placements.extend(ifo.event_objects.iter().map(|o| {
                ("EVENT", TableRef::Event, o.object.object_id as usize, &o.object)
            }));
            placements.extend(
                ifo.collision_objects
                    .iter()
                    .map(|o| ("COLLISION", TableRef::Special, SPECIAL_COLLISION_OBJECT, o)),
            );
            placements.extend(
                ifo.warps
                    .iter()
                    .map(|o| ("WARP", TableRef::Special, SPECIAL_WARP_OBJECT, o)),
            );

            for (kind, table, object_id, object) in placements {
                let zsc = match table {
                    TableRef::Zone(zsc) => Some(zsc),
                    TableRef::Special => special.as_ref(),
                    TableRef::Event => event.as_ref(),
                };
                let Some(zsc) = zsc else {
                    continue;
                };
                let Some(zsc_object) = zsc.objects.get(object_id) else {
                    continue;
                };
                let object_transform = collector.object_transform(object);
                for (part_index, part) in zsc_object.parts.iter().enumerate() {
                    let Some(mesh_path) = zsc.meshes.get(part.mesh_id as usize) else {
                        continue;
                    };
                    let part_transform = Affine3A::from_scale_rotation_translation(
                        scale(part.scale.x, part.scale.y, part.scale.z),
                        quat(part.rotation.x, part.rotation.y, part.rotation.z, part.rotation.w),
                        Vec3::new(part.position.x, part.position.y, part.position.z),
                    );
                    let transform = object_transform * part_transform;
                    let Some(triangles) = assets.mesh_triangles(vfs, mesh_path) else {
                        lines.push(format!(
                            "block {}_{} {} obj {} part {}: mesh {} UNREADABLE",
                            bx,
                            by,
                            kind,
                            object_id,
                            part_index,
                            mesh_path.path().display()
                        ));
                        continue;
                    };
                    let mut min = Vec3::splat(f32::INFINITY);
                    let mut max = Vec3::splat(f32::NEG_INFINITY);
                    for tri in triangles.iter() {
                        for v in tri {
                            let w = transform.transform_point3(*v);
                            min = min.min(w);
                            max = max.max(w);
                        }
                    }
                    let nearest = center.clamp(min.truncate(), max.truncate());
                    if nearest.distance(center) > radius {
                        continue;
                    }
                    let path = mesh_path.path().to_string_lossy();
                    let role = match (kind, part.collision_shape) {
                        ("COLLISION", _) => "wall",
                        ("WARP", _) => "warp keep-out",
                        (_, None) => "ignored (no collision shape)",
                        _ if is_walk_through_mesh(&path) => "ignored (walk-through grass)",
                        _ => {
                            let flags = part.collision_flags;
                            match (
                                kind != "EVENT" && !flags.contains(ZscCollisionFlags::HEIGHT_ONLY),
                                !flags.contains(ZscCollisionFlags::NOT_MOVEABLE),
                            ) {
                                (true, true) => "floor + steep faces wall",
                                (true, false) => "wall",
                                (false, true) => "floor only",
                                (false, false) => "ignored",
                            }
                        }
                    };
                    lines.push(format!(
                        "block {}_{} {} obj {} part {} mesh {} shape {:?} flags {:?} -> {}; x {:.0}..{:.0} y {:.0}..{:.0} z {:.0}..{:.0}",
                        bx,
                        by,
                        kind,
                        object_id,
                        part_index,
                        path,
                        part.collision_shape,
                        part.collision_flags,
                        role,
                        min.x,
                        max.x,
                        min.y,
                        max.y,
                        min.z,
                        max.z,
                    ));
                }
            }
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grass_kind_meshes_are_walk_through() {
        assert!(is_walk_through_mesh("3DDATA/JUNON/DECO/GRASS002.ZMS"));
        assert!(is_walk_through_mesh("3ddata/junon/deco/bush01.zms"));
        assert!(!is_walk_through_mesh("3ddata/junon/deco/tree_leaves_bush.zms"));
        assert!(!is_walk_through_mesh("3DDATA/JUNON/DECO/TREE004B.ZMS"));
        assert!(!is_walk_through_mesh("3DDATA/JUNON/CNST/CASTLE01.ZMS"));
    }

    #[test]
    fn degenerate_rotation_and_scale_fall_back() {
        assert_eq!(quat(0.0, 0.0, 0.0, 0.0), Quat::IDENTITY);
        assert_eq!(scale(0.0, 2.0, 0.0), Vec3::new(1.0, 2.0, 1.0));
    }
}
