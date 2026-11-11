//! 2.5D walkability bake: turns a zone's terrain, collision geometry and water
//! into a [`NavGrid`] with blocked reasons and floor heights.
//!
//! Pure geometry; `rose-data-irose` reads the game files and fills
//! [`NavBakeInput`]. The model follows the client's movement rules:
//!
//! - **Terrain** is the HIM heightmap, sampled bilinearly at cell centers.
//!   Terrain steeper than `max_slope_deg` blocks where it is the floor
//!   (cliffs, river and moat banks).
//! - **Object floors** are the flat triangles of collision parts that accept
//!   movement (no `NOT_MOVEABLE`): bridges, decks, ramps, castle gate
//!   platforms. Every object cell holds a stack of candidate surfaces
//!   (terrain plus merged object floors); one is standable with `body_cm` of
//!   headroom and no wall in its body band.
//! - **Walls** are collidable parts that are `NOT_MOVEABLE` (house walls,
//!   trunks, castle walls) plus the steep faces of any collidable part. A
//!   wall blocks only where it enters the body band (floor + `step_cm` to
//!   floor + `body_cm`), so low curbs and overhead roofs do not block.
//! - **Reachability**: surfaces of 4-adjacent cells link when their heights
//!   differ by at most a step (plus `slope_allow` per cell of run). A cell's
//!   floor is its highest surface linked to open ground, so decks reached by
//!   ramps are walkable and pillar or wall tops are not.
//! - **Ledges**: a cell next to an object floor more than a step higher
//!   blocks (no stepping up onto platforms from the side, no dropping off).
//! - **Water** deeper than `wade_depth_cm` above the floor blocks, and warp
//!   gates get a keep-out box.
//!
//! Grid row 0 is the south edge (smallest y), as in [`NavGrid`].

use std::time::Instant;

use bevy::math::{Vec2, Vec3};

use crate::nav_grid::{
    NavGrid, NAV_BLOCK_CLEARANCE, NAV_BLOCK_LEDGE, NAV_BLOCK_NO_FLOOR, NAV_BLOCK_SLOPE,
    NAV_BLOCK_VOID, NAV_BLOCK_WALL, NAV_BLOCK_WARP, NAV_BLOCK_WATER,
};

/// Model constants (cm, degrees).
#[derive(Clone, Debug)]
pub struct NavBakeParams {
    /// Preferred cell edge; grown by doubling past `max_cells_per_axis`.
    pub cell_cm: f32,
    pub max_cells_per_axis: u32,
    /// Steepest walkable triangle (live-calibrated on the original client).
    pub max_slope_deg: f32,
    /// Highest step up between neighbouring floors, and the bottom of the
    /// body band walls are tested against.
    pub step_cm: f32,
    /// Body height: geometry above floor + this is overhead.
    pub body_cm: f32,
    /// Floor samples closer than this in one cell are one surface.
    pub merge_dz_cm: f32,
    /// Extra rise allowed per cm of horizontal run between neighbouring
    /// object floors (ramps).
    pub slope_allow: f32,
    /// Water deeper than this above the floor blocks.
    pub wade_depth_cm: f32,
    /// Free margin kept around blocked cells (only applied when it spans at
    /// least one cell).
    pub agent_radius_cm: f32,
    /// Surfaces kept per cell (lowest first).
    pub max_layers: usize,
}

impl Default for NavBakeParams {
    fn default() -> Self {
        Self {
            cell_cm: 100.0,
            max_cells_per_axis: 4096,
            max_slope_deg: 62.0,
            step_cm: 50.0,
            body_cm: 180.0,
            merge_dz_cm: 20.0,
            slope_allow: 0.5,
            wade_depth_cm: 100.0,
            agent_radius_cm: 40.0,
            max_layers: 6,
        }
    }
}

/// One HIM heightmap block. Row 0 is the NORTH edge.
#[derive(Clone, Debug)]
pub struct NavTerrainBlock {
    /// World x (cm) of column 0.
    pub west_x: f32,
    /// World y (cm) of row 0.
    pub north_y: f32,
    /// Sample spacing (cm).
    pub spacing: f32,
    pub width: u32,
    pub height: u32,
    /// Row-major heights (cm).
    pub heights: Vec<f32>,
}

impl NavTerrainBlock {
    fn height_at(&self, col: usize, row: usize) -> f32 {
        self.heights[row * self.width as usize + col]
    }
}

/// A world-space collision triangle (cm) with the movement rules of its part.
#[derive(Clone, Copy, Debug)]
pub struct NavBakeTriangle {
    pub vertices: [Vec3; 3],
    /// Blocks movement: its `NOT_MOVEABLE` or steep faces are walls.
    pub collidable: bool,
    /// Accepts movement: its flat faces are floors.
    pub moveable: bool,
}

/// An IFO water plane (world cm).
#[derive(Clone, Copy, Debug)]
pub struct NavWaterPlane {
    pub min: Vec2,
    pub max: Vec2,
    pub z: f32,
}

#[derive(Clone, Debug, Default)]
pub struct NavBakeInput {
    /// World bounds (cm) of the grid.
    pub min: Vec2,
    pub max: Vec2,
    pub terrain: Vec<NavTerrainBlock>,
    pub triangles: Vec<NavBakeTriangle>,
    pub water: Vec<NavWaterPlane>,
    /// Axis-aligned keep-out boxes (warp gates).
    pub keep_out: Vec<(Vec2, Vec2)>,
    /// Designer placed walk goals (start, revive, NPC, spawn points).
    pub anchors: Vec<Vec3>,
}

/// What the bake found, for logs and the map tool.
#[derive(Clone, Debug, Default)]
pub struct NavBakeReport {
    pub width: u32,
    pub height: u32,
    pub cell_cm: f32,
    pub floor_triangles: usize,
    pub wall_triangles: usize,
    pub object_cells: usize,
    pub surfaces: usize,
    pub reachable_surfaces: usize,
    pub object_floor_cells: usize,
    pub free_cells: usize,
    pub wall_cells: usize,
    pub slope_cells: usize,
    pub ledge_cells: usize,
    pub no_floor_cells: usize,
    pub void_cells: usize,
    pub water_cells: usize,
    pub warp_cells: usize,
    pub clearance_cells: usize,
    pub millis: u128,
}

const NO_OBJECT_CELL: u32 = u32::MAX;

/// Union-find over surfaces plus the open-ground node.
struct DisjointSet {
    parent: Vec<u32>,
}

impl DisjointSet {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len as u32).collect(),
        }
    }

    fn find(&mut self, mut i: u32) -> u32 {
        while self.parent[i as usize] != i {
            let grand = self.parent[self.parent[i as usize] as usize];
            self.parent[i as usize] = grand;
            i = grand;
        }
        i
    }

    fn union(&mut self, a: u32, b: u32) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[ra as usize] = rb;
        }
    }
}

/// Cell grid geometry shared by the passes.
struct Raster {
    origin: Vec2,
    cell: f32,
    width: usize,
    height: usize,
}

impl Raster {
    fn center(&self, cx: usize, cy: usize) -> Vec2 {
        self.origin + Vec2::new((cx as f32 + 0.5) * self.cell, (cy as f32 + 0.5) * self.cell)
    }

    /// Inclusive cell range whose centers lie in `[lo, hi]` (world cm).
    fn span(&self, lo: f32, hi: f32, origin: f32, cells: usize) -> Option<(usize, usize)> {
        let first = ((lo - origin) / self.cell - 0.5).ceil().max(0.0);
        let last = ((hi - origin) / self.cell - 0.5).floor();
        if last < first || first >= cells as f32 || last < 0.0 {
            return None;
        }
        Some((first as usize, (last as usize).min(cells - 1)))
    }

    fn cells_in(&self, min: Vec2, max: Vec2) -> Option<((usize, usize), (usize, usize))> {
        Some((
            self.span(min.x, max.x, self.origin.x, self.width)?,
            self.span(min.y, max.y, self.origin.y, self.height)?,
        ))
    }
}

fn is_steep(t: &[Vec3; 3], cos_max: f32) -> bool {
    let n = (t[1] - t[0]).cross(t[2] - t[0]);
    let len = n.length();
    len <= 1e-6 || n.z.abs() < cos_max * len
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let s = if len2 > 1e-12 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * s)
}

/// Whether `p` is inside the triangle's footprint or within `pad` of it.
fn covers(t: &[Vec3; 3], p: Vec2, pad: f32) -> bool {
    let (a, b, c) = (t[0].truncate(), t[1].truncate(), t[2].truncate());
    let d1 = (p.x - b.x) * (a.y - b.y) - (a.x - b.x) * (p.y - b.y);
    let d2 = (p.x - c.x) * (b.y - c.y) - (b.x - c.x) * (p.y - c.y);
    let d3 = (p.x - a.x) * (c.y - a.y) - (c.x - a.x) * (p.y - a.y);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    if !(has_neg && has_pos) {
        return true;
    }
    segment_distance(p, a, b) <= pad
        || segment_distance(p, b, c) <= pad
        || segment_distance(p, c, a) <= pad
}

/// Height of a flat triangle's plane at `p`, clamped to its z range.
fn plane_z(t: &[Vec3; 3], p: Vec2) -> f32 {
    let n = (t[1] - t[0]).cross(t[2] - t[0]);
    let zmin = t[0].z.min(t[1].z).min(t[2].z);
    let zmax = t[0].z.max(t[1].z).max(t[2].z);
    if n.z.abs() <= 1e-9 {
        return zmax;
    }
    let z = t[0].z - (n.x * (p.x - t[0].x) + n.y * (p.y - t[0].y)) / n.z;
    z.clamp(zmin, zmax)
}

/// Vertical extent of a steep triangle along the vertical line through the
/// closest point of its footprint: project onto the footprint's longest
/// edge direction `s` and intersect the line `s = s0` with the `(s, z)`
/// triangle.
fn wall_z_range(t: &[Vec3; 3], p: Vec2) -> (f32, f32) {
    let zmin = t[0].z.min(t[1].z).min(t[2].z);
    let zmax = t[0].z.max(t[1].z).max(t[2].z);
    let edges = [
        t[1].truncate() - t[0].truncate(),
        t[2].truncate() - t[1].truncate(),
        t[0].truncate() - t[2].truncate(),
    ];
    let longest = edges
        .iter()
        .copied()
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
        .unwrap_or(Vec2::ZERO);
    let Some(dir) = longest.try_normalize() else {
        return (zmin, zmax);
    };
    let origin = t[0].truncate();
    let s = [
        0.0,
        (t[1].truncate() - origin).dot(dir),
        (t[2].truncate() - origin).dot(dir),
    ];
    let smin = s[0].min(s[1]).min(s[2]);
    let smax = s[0].max(s[1]).max(s[2]);
    let s0 = (p - origin).dot(dir).clamp(smin, smax);
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for (i, j) in [(0, 1), (1, 2), (2, 0)] {
        let (s1, z1, s2, z2) = (s[i], t[i].z, s[j], t[j].z);
        if s0 < s1.min(s2) - 1e-3 || s0 > s1.max(s2) + 1e-3 {
            continue;
        }
        let ds = s2 - s1;
        if ds.abs() <= 1e-3 {
            lo = lo.min(z1.min(z2));
            hi = hi.max(z1.max(z2));
        } else {
            let z = z1 + ((s0 - s1) / ds).clamp(0.0, 1.0) * (z2 - z1);
            lo = lo.min(z);
            hi = hi.max(z);
        }
    }
    if lo.is_finite() {
        (lo, hi)
    } else {
        (zmin, zmax)
    }
}

/// Bilinear terrain height and steep-quad mask at every cell center.
fn bake_terrain(raster: &Raster, blocks: &[NavTerrainBlock], cos_max: f32) -> (Vec<f32>, Vec<bool>) {
    let cells = raster.width * raster.height;
    let mut terrain = vec![f32::NAN; cells];
    let mut steep = vec![false; cells];
    for block in blocks {
        let (w, h) = (block.width as usize, block.height as usize);
        if w < 2 || h < 2 || block.heights.len() < w * h || block.spacing <= 0.0 {
            continue;
        }
        let sp = block.spacing;
        // A quad is steep when any triangle of either diagonal split is.
        let mut quad_steep = vec![false; (w - 1) * (h - 1)];
        for row in 0..h - 1 {
            for col in 0..w - 1 {
                let p00 = Vec3::new(0.0, 0.0, block.height_at(col, row));
                let p10 = Vec3::new(sp, 0.0, block.height_at(col + 1, row));
                let p01 = Vec3::new(0.0, -sp, block.height_at(col, row + 1));
                let p11 = Vec3::new(sp, -sp, block.height_at(col + 1, row + 1));
                quad_steep[row * (w - 1) + col] = [
                    [p00, p10, p11],
                    [p00, p11, p01],
                    [p00, p10, p01],
                    [p10, p11, p01],
                ]
                .iter()
                .any(|t| is_steep(t, cos_max));
            }
        }

        let east_x = block.west_x + (w - 1) as f32 * sp;
        let south_y = block.north_y - (h - 1) as f32 * sp;
        let Some(((cx0, cx1), (cy0, cy1))) = raster.cells_in(
            Vec2::new(block.west_x, south_y),
            Vec2::new(east_x, block.north_y),
        ) else {
            continue;
        };
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let p = raster.center(cx, cy);
                let fx = ((p.x - block.west_x) / sp).clamp(0.0, (w - 1) as f32 - 1e-4);
                let fr = ((block.north_y - p.y) / sp).clamp(0.0, (h - 1) as f32 - 1e-4);
                let (col, row) = (fx as usize, fr as usize);
                let (u, v) = (fx - col as f32, fr - row as f32);
                let i = cy * raster.width + cx;
                terrain[i] = block.height_at(col, row) * (1.0 - u) * (1.0 - v)
                    + block.height_at(col + 1, row) * u * (1.0 - v)
                    + block.height_at(col, row + 1) * (1.0 - u) * v
                    + block.height_at(col + 1, row + 1) * u * v;
                steep[i] = quad_steep[row * (w - 1) + col];
            }
        }
    }
    (terrain, steep)
}

/// Bake the walkability grid of one zone.
pub fn bake_nav_grid(input: &NavBakeInput, params: &NavBakeParams) -> (NavGrid, NavBakeReport) {
    let started = Instant::now();
    let span = (input.max - input.min).max(Vec2::splat(params.cell_cm));
    let mut cell = params.cell_cm.max(25.0);
    while span.x / cell > params.max_cells_per_axis as f32
        || span.y / cell > params.max_cells_per_axis as f32
    {
        cell *= 2.0;
    }
    let raster = Raster {
        origin: input.min,
        cell,
        width: ((span.x / cell).ceil() as usize).max(1),
        height: ((span.y / cell).ceil() as usize).max(1),
    };
    let (nx, ny) = (raster.width, raster.height);
    let cells = nx * ny;
    let cos_max = params.max_slope_deg.to_radians().cos();
    let step = params.step_cm;
    let body = params.body_cm;
    let link_dz = params.step_cm + cell * params.slope_allow;
    let pad = cell * 0.5;
    let mut report = NavBakeReport {
        width: nx as u32,
        height: ny as u32,
        cell_cm: cell,
        ..Default::default()
    };

    // 1. Terrain.
    let (terrain, terrain_steep) = bake_terrain(&raster, &input.terrain, cos_max);

    // 2. Rasterize collision triangles: floor samples and wall spans.
    let mut floor_samples: Vec<(u32, f32)> = Vec::new();
    let mut walls: Vec<(u32, f32, f32)> = Vec::new();
    for tri in input.triangles.iter() {
        let t = &tri.vertices;
        if !t.iter().all(|v| v.is_finite()) {
            continue;
        }
        let steep = is_steep(t, cos_max);
        let floor = tri.moveable && !steep;
        let wall = tri.collidable && (!tri.moveable || steep);
        if !floor && !wall {
            continue;
        }
        if floor {
            report.floor_triangles += 1;
        } else {
            report.wall_triangles += 1;
        }
        let min = t[0].truncate().min(t[1].truncate()).min(t[2].truncate()) - Vec2::splat(pad);
        let max = t[0].truncate().max(t[1].truncate()).max(t[2].truncate()) + Vec2::splat(pad);
        let Some(((cx0, cx1), (cy0, cy1))) = raster.cells_in(min, max) else {
            continue;
        };
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let p = raster.center(cx, cy);
                if !covers(t, p, pad) {
                    continue;
                }
                let i = cy * nx + cx;
                if floor {
                    let z = plane_z(t, p);
                    // Surfaces buried in the terrain are not floors.
                    if terrain[i].is_nan() || z > terrain[i] - step {
                        floor_samples.push((i as u32, z));
                    }
                } else {
                    let (lo, hi) = wall_z_range(t, p);
                    walls.push((i as u32, lo, hi));
                }
            }
        }
    }

    // 3. Surfaces per object cell: terrain + merged floor samples.
    let mut sampled_cells: Vec<u32> = floor_samples.iter().map(|&(i, _)| i).collect();
    sampled_cells.sort_unstable();
    sampled_cells.dedup();
    for i in sampled_cells {
        if !terrain[i as usize].is_nan() {
            floor_samples.push((i, terrain[i as usize]));
        }
    }
    floor_samples.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut object_cell = vec![NO_OBJECT_CELL; cells];
    let mut cell_ids: Vec<u32> = Vec::new();
    let mut cell_start: Vec<u32> = Vec::new();
    let mut surface_z: Vec<f32> = Vec::new();
    let mut k = 0;
    while k < floor_samples.len() {
        let cell_id = floor_samples[k].0;
        let first_surface = surface_z.len();
        object_cell[cell_id as usize] = cell_ids.len() as u32;
        cell_ids.push(cell_id);
        cell_start.push(first_surface as u32);
        let mut previous = f32::NEG_INFINITY;
        // Whether the current merge group got a layer (layers are capped).
        let mut group_kept = false;
        while k < floor_samples.len() && floor_samples[k].0 == cell_id {
            let z = floor_samples[k].1;
            if z - previous > params.merge_dz_cm {
                group_kept = surface_z.len() - first_surface < params.max_layers;
                if group_kept {
                    surface_z.push(z);
                }
            } else if group_kept {
                // Merged group: its top is the floor.
                if let Some(last) = surface_z.last_mut() {
                    *last = last.max(z);
                }
            }
            previous = z;
            k += 1;
        }
    }
    cell_start.push(surface_z.len() as u32);
    drop(floor_samples);
    let surfaces = surface_z.len();
    report.object_cells = cell_ids.len();
    report.surfaces = surfaces;
    let surface_range = |oc: usize| cell_start[oc] as usize..cell_start[oc + 1] as usize;

    // Headroom: the next surface up must leave a body height.
    let mut standable = vec![true; surfaces];
    for oc in 0..cell_ids.len() {
        let range = surface_range(oc);
        for s in range.start..range.end.saturating_sub(1) {
            if surface_z[s + 1] - surface_z[s] < body {
                standable[s] = false;
            }
        }
    }

    // 4. Walls against the terrain band and every surface's body band.
    let mut terrain_wall = vec![false; cells];
    let mut cell_walled = vec![false; cells];
    for &(i, lo, hi) in walls.iter() {
        let i = i as usize;
        let t = terrain[i];
        if !t.is_nan() && hi > t + step && lo < t + body {
            terrain_wall[i] = true;
            cell_walled[i] = true;
        }
        let oc = object_cell[i];
        if oc != NO_OBJECT_CELL {
            for s in surface_range(oc as usize) {
                let z = surface_z[s];
                if hi > z + step && lo < z + body {
                    standable[s] = false;
                    cell_walled[i] = true;
                }
            }
        }
    }
    drop(walls);

    // 5. Reachability of object surfaces from open ground.
    let ground = surfaces as u32;
    let mut sets = DisjointSet::new(surfaces + 1);
    let open_ground =
        |i: usize| !terrain[i].is_nan() && !terrain_wall[i] && !terrain_steep[i];
    for (oc, &cell_id) in cell_ids.iter().enumerate() {
        let i = cell_id as usize;
        let (cx, cy) = (i % nx, i / nx);
        for s in surface_range(oc) {
            if standable[s] && open_ground(i) && (surface_z[s] - terrain[i]).abs() <= params.merge_dz_cm
            {
                sets.union(s as u32, ground);
            }
        }
        for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let (ncx, ncy) = (cx as i32 + dx, cy as i32 + dy);
            if ncx < 0 || ncy < 0 || ncx >= nx as i32 || ncy >= ny as i32 {
                continue;
            }
            let n = ncy as usize * nx + ncx as usize;
            let noc = object_cell[n];
            if noc == NO_OBJECT_CELL {
                if !open_ground(n) {
                    continue;
                }
                for s in surface_range(oc) {
                    if standable[s] && (surface_z[s] - terrain[n]).abs() <= link_dz {
                        sets.union(s as u32, ground);
                    }
                }
            } else {
                for s in surface_range(oc) {
                    if !standable[s] {
                        continue;
                    }
                    for t in surface_range(noc as usize) {
                        if standable[t] && (surface_z[s] - surface_z[t]).abs() <= link_dz {
                            sets.union(s as u32, t as u32);
                        }
                    }
                }
            }
        }
    }
    let ground_root = sets.find(ground);

    // 6. Per-cell floor and flags.
    let mut flags = vec![0u8; cells];
    let mut floor = terrain.clone();
    let mut object_floor = vec![false; cells];
    for (oc, &cell_id) in cell_ids.iter().enumerate() {
        let i = cell_id as usize;
        let mut best = f32::NEG_INFINITY;
        for s in surface_range(oc) {
            if standable[s] && sets.find(s as u32) == ground_root {
                report.reachable_surfaces += 1;
                best = best.max(surface_z[s]);
            }
        }
        if best.is_finite() {
            floor[i] = best;
            object_floor[i] = terrain[i].is_nan() || best > terrain[i] + params.merge_dz_cm;
        } else if cell_walled[i] {
            flags[i] |= NAV_BLOCK_WALL;
        } else {
            flags[i] |= NAV_BLOCK_NO_FLOOR;
        }
    }
    for i in 0..cells {
        if object_cell[i] == NO_OBJECT_CELL && terrain_wall[i] {
            flags[i] |= NAV_BLOCK_WALL;
        }
        if terrain_steep[i] && !object_floor[i] {
            flags[i] |= NAV_BLOCK_SLOPE;
        }
        if terrain[i].is_nan() && !object_floor[i] {
            flags[i] |= NAV_BLOCK_VOID;
        }
    }
    report.object_floor_cells = object_floor.iter().filter(|&&o| o).count();

    // 7. Ledges: a neighbouring object floor (or this one) more than a
    // step plus ramp allowance higher. Terrain-terrain pairs are the slope
    // rule's job.
    for cy in 0..ny {
        for cx in 0..nx {
            let i = cy * nx + cx;
            if floor[i].is_nan() {
                continue;
            }
            'neighbours: for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let (ncx, ncy) = (cx as i32 + dx, cy as i32 + dy);
                    if ncx < 0 || ncy < 0 || ncx >= nx as i32 || ncy >= ny as i32 {
                        continue;
                    }
                    let n = ncy as usize * nx + ncx as usize;
                    if (object_floor[n] || object_floor[i]) && floor[n] > floor[i] + link_dz {
                        flags[i] |= NAV_BLOCK_LEDGE;
                        break 'neighbours;
                    }
                }
            }
        }
    }

    // 8. Deep water.
    for plane in input.water.iter() {
        let Some(((cx0, cx1), (cy0, cy1))) = raster.cells_in(plane.min, plane.max) else {
            continue;
        };
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                let i = cy * nx + cx;
                if !floor[i].is_nan() && plane.z - floor[i] > params.wade_depth_cm {
                    flags[i] |= NAV_BLOCK_WATER;
                }
            }
        }
    }

    // 9. Warp gate keep-out.
    for &(min, max) in input.keep_out.iter() {
        let Some(((cx0, cx1), (cy0, cy1))) = raster.cells_in(min, max) else {
            continue;
        };
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                flags[cy * nx + cx] |= NAV_BLOCK_WARP;
            }
        }
    }

    // 10. Body clearance around blocked cells (Euclidean, from the original
    // blocked set so it does not cascade).
    let radius_cells = (params.agent_radius_cm / cell).floor() as i32;
    if radius_cells >= 1 {
        let blocked_before: Vec<bool> = flags.iter().map(|&f| f != 0).collect();
        let r2 = (params.agent_radius_cm / cell).powi(2);
        for cy in 0..ny as i32 {
            for cx in 0..nx as i32 {
                if !blocked_before[cy as usize * nx + cx as usize] {
                    continue;
                }
                for dy in -radius_cells..=radius_cells {
                    for dx in -radius_cells..=radius_cells {
                        let (ncx, ncy) = (cx + dx, cy + dy);
                        if (dx * dx + dy * dy) as f32 > r2
                            || ncx < 0
                            || ncy < 0
                            || ncx >= nx as i32
                            || ncy >= ny as i32
                        {
                            continue;
                        }
                        let n = ncy as usize * nx + ncx as usize;
                        if !blocked_before[n] {
                            flags[n] |= NAV_BLOCK_CLEARANCE;
                        }
                    }
                }
            }
        }
    }

    for &f in flags.iter() {
        if f == 0 {
            report.free_cells += 1;
        }
        report.wall_cells += (f & NAV_BLOCK_WALL != 0) as usize;
        report.slope_cells += (f & NAV_BLOCK_SLOPE != 0) as usize;
        report.ledge_cells += (f & NAV_BLOCK_LEDGE != 0) as usize;
        report.no_floor_cells += (f & NAV_BLOCK_NO_FLOOR != 0) as usize;
        report.void_cells += (f & NAV_BLOCK_VOID != 0) as usize;
        report.water_cells += (f & NAV_BLOCK_WATER != 0) as usize;
        report.warp_cells += (f & NAV_BLOCK_WARP != 0) as usize;
        report.clearance_cells += (f & NAV_BLOCK_CLEARANCE != 0) as usize;
    }

    let mut grid = NavGrid::new(input.min.x, input.min.y, cell, nx as u32, ny as u32);
    grid.blocked = flags;
    grid.set_floor_heights(&floor);
    grid.set_anchors(input.anchors.iter().map(|a| (a.x, a.y)).collect());
    report.millis = started.elapsed().as_millis();
    (grid, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav_grid::NAV_BLOCK_WALL;

    /// One flat 62.5 m block at height `z` covering x/y 0..6250.
    fn flat_block(z: f32) -> NavTerrainBlock {
        NavTerrainBlock {
            west_x: 0.0,
            north_y: 6250.0,
            spacing: 250.0,
            width: 26,
            height: 26,
            heights: vec![z; 26 * 26],
        }
    }

    fn input_with(terrain: NavTerrainBlock) -> NavBakeInput {
        NavBakeInput {
            min: Vec2::ZERO,
            max: Vec2::splat(6250.0),
            terrain: vec![terrain],
            ..Default::default()
        }
    }

    /// Axis-aligned box (two triangles per face, outward winding unused).
    fn boxed(min: Vec3, max: Vec3, collidable: bool, moveable: bool) -> Vec<NavBakeTriangle> {
        let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let (a, b) = (min, max);
        let quads = [
            [c(a.x, a.y, b.z), c(b.x, a.y, b.z), c(b.x, b.y, b.z), c(a.x, b.y, b.z)],
            [c(a.x, a.y, a.z), c(b.x, a.y, a.z), c(b.x, a.y, b.z), c(a.x, a.y, b.z)],
            [c(a.x, b.y, a.z), c(b.x, b.y, a.z), c(b.x, b.y, b.z), c(a.x, b.y, b.z)],
            [c(a.x, a.y, a.z), c(a.x, b.y, a.z), c(a.x, b.y, b.z), c(a.x, a.y, b.z)],
            [c(b.x, a.y, a.z), c(b.x, b.y, a.z), c(b.x, b.y, b.z), c(b.x, a.y, b.z)],
        ];
        quads
            .iter()
            .flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
            .map(|vertices| NavBakeTriangle {
                vertices,
                collidable,
                moveable,
            })
            .collect()
    }

    #[test]
    fn flat_terrain_is_free_with_floor_heights() {
        let (grid, report) = bake_nav_grid(&input_with(flat_block(300.0)), &NavBakeParams::default());
        assert_eq!(report.free_cells, (grid.width * grid.height) as usize);
        assert_eq!(grid.floor_height_world(3000.0, 3000.0), Some(300.0));
    }

    #[test]
    fn cliff_blocks_as_slope() {
        // Terrain jumps 10 m between two sample columns at x = 2500..2750.
        let mut block = flat_block(0.0);
        for row in 0..26 {
            for col in 11..26 {
                block.heights[row * 26 + col] = 1000.0;
            }
        }
        let (grid, _) = bake_nav_grid(&input_with(block), &NavBakeParams::default());
        assert!(grid.flags_world(2600.0, 3000.0) & NAV_BLOCK_SLOPE != 0);
        assert_eq!(grid.flags_world(1000.0, 3000.0), 0);
        assert_eq!(grid.flags_world(4000.0, 3000.0), 0);
        assert!(grid.find_path(1000.0, 3000.0, 4000.0, 3000.0).is_none());
    }

    #[test]
    fn not_moveable_box_is_a_wall_and_low_box_is_not() {
        let mut input = input_with(flat_block(0.0));
        input.triangles = boxed(
            Vec3::new(2000.0, 2000.0, 0.0),
            Vec3::new(2400.0, 2400.0, 500.0),
            true,
            false,
        );
        // A 30 cm curb accepting movement: steps over it.
        input.triangles.extend(boxed(
            Vec3::new(4000.0, 4000.0, 0.0),
            Vec3::new(4400.0, 4400.0, 30.0),
            true,
            true,
        ));
        let (grid, _) = bake_nav_grid(&input, &NavBakeParams::default());
        // The box's sides block; its overhead top does not, so the enclosed
        // interior is a free but cut off pocket.
        assert!(grid.flags_world(2000.0, 2200.0) & NAV_BLOCK_WALL != 0);
        assert!(grid.flags_world(2400.0, 2200.0) & NAV_BLOCK_WALL != 0);
        assert!(grid.find_path(500.0, 500.0, 2200.0, 2200.0).map_or(true, |path| {
            let (x, y) = *path.last().unwrap();
            (x - 2200.0).hypot(y - 2200.0) > 100.0
        }));
        assert_eq!(grid.flags_world(4200.0, 4200.0), 0);
        assert_eq!(grid.floor_height_world(4200.0, 4200.0), Some(30.0));
    }

    #[test]
    fn ramp_reaches_platform_and_sides_are_ledges() {
        let mut input = input_with(flat_block(0.0));
        // 3 m high platform at x 3000..4000, y 2000..4000.
        input.triangles = boxed(
            Vec3::new(3000.0, 2000.0, 0.0),
            Vec3::new(4000.0, 4000.0, 300.0),
            true,
            true,
        );
        // Ramp along x from 1500 (z 0) to 3000 (z 300) at y 2800..3200: 11 deg.
        let r = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        for vertices in [
            [r(1500.0, 2800.0, 0.0), r(3000.0, 2800.0, 300.0), r(3000.0, 3200.0, 300.0)],
            [r(1500.0, 2800.0, 0.0), r(3000.0, 3200.0, 300.0), r(1500.0, 3200.0, 0.0)],
        ] {
            input.triangles.push(NavBakeTriangle {
                vertices,
                collidable: true,
                moveable: true,
            });
        }
        let (grid, report) = bake_nav_grid(&input, &NavBakeParams::default());
        assert!(report.object_floor_cells > 0);
        assert_eq!(grid.flags_world(3500.0, 3000.0), 0, "platform top is walkable");
        assert_eq!(grid.floor_height_world(3500.0, 3000.0), Some(300.0));
        // Beside the platform (not at the ramp) the ground is a ledge.
        assert!(grid.flags_world(2850.0, 2300.0) & NAV_BLOCK_LEDGE != 0);
        assert!(grid.flags_world(1000.0, 2300.0) == 0);
        // The ramp connects ground and platform.
        assert!(grid.find_path(500.0, 3000.0, 3500.0, 3000.0).is_some());
    }

    #[test]
    fn deep_water_blocks_but_shallow_does_not() {
        let mut block = flat_block(0.0);
        // A 3 m deep trench (gentle banks are not the point here).
        for row in 0..26 {
            for col in 0..26 {
                if (10..=14).contains(&col) {
                    block.heights[row * 26 + col] = -300.0;
                }
            }
        }
        let mut input = input_with(block);
        input.water.push(NavWaterPlane {
            min: Vec2::new(0.0, 0.0),
            max: Vec2::new(6400.0, 6400.0),
            z: 50.0,
        });
        let (grid, _) = bake_nav_grid(&input, &NavBakeParams::default());
        assert!(grid.flags_world(3000.0, 3000.0) & NAV_BLOCK_WATER != 0);
        assert_eq!(grid.flags_world(500.0, 3000.0) & NAV_BLOCK_WATER, 0);
    }

    #[test]
    fn unanchored_pocket_is_stranded_and_rescued() {
        // Square pit 10 m deep: its floor is free but cut off by slopes.
        let mut block = flat_block(0.0);
        for row in 8..=16 {
            for col in 8..=16 {
                block.heights[row * 26 + col] = -1000.0;
            }
        }
        let mut input = input_with(block);
        input.anchors.push(Vec3::new(500.0, 500.0, 0.0));
        let (grid, _) = bake_nav_grid(&input, &NavBakeParams::default());
        // Pit centre: x = 12 * 250, y = 6250 - 12 * 250.
        let pit = Vec3::new(3000.0, 3250.0, -1000.0);
        assert_eq!(grid.flags_world(pit.x, pit.y), 0);
        assert!(grid.is_stranded(pit.x, pit.y));
        assert!(!grid.is_stranded(500.0, 500.0));
        let rescue = grid.rescue_point(pit, 3000.0).expect("rescue");
        assert!(!grid.is_stranded(rescue.x, rescue.y));
        // Destinations picked from the pit lead out of it.
        let goal = grid
            .reachable_point(pit, pit + Vec3::new(100.0, 0.0, 0.0), 3000.0)
            .expect("goal");
        assert!(!grid.is_stranded(goal.x, goal.y));
        // Destinations inside the pit picked from outside are moved out.
        let outside = Vec3::new(500.0, 500.0, 0.0);
        let snapped = grid.reachable_point(outside, pit, 3000.0).expect("snapped");
        assert!(!grid.is_stranded(snapped.x, snapped.y));
        assert_eq!(grid.component_at_world(snapped.x, snapped.y), grid.component_at_world(500.0, 500.0));
    }

    #[test]
    fn warp_keep_out_blocks() {
        let mut input = input_with(flat_block(0.0));
        input
            .keep_out
            .push((Vec2::new(1000.0, 1000.0), Vec2::new(1500.0, 1500.0)));
        let (grid, _) = bake_nav_grid(&input, &NavBakeParams::default());
        assert!(grid.flags_world(1250.0, 1250.0) & NAV_BLOCK_WARP != 0);
        assert_eq!(grid.flags_world(2000.0, 2000.0), 0);
    }
}
