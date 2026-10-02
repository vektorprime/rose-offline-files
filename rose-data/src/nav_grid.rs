//! Server-side 2D navigation grid for bot pathfinding.
//!
//! Bots (big-brain thinkers) and monster AI issue straight-line
//! `CommandData::Move` orders with no knowledge of buildings, walls or props.
//! The client avoids these with Rapier shape-casts, but the headless server
//! has no physics world. This grid is baked at zone load from the same source
//! data the client builds colliders from (IFO instances + ZSC collision
//! shapes + ZMS mesh bounds) and gives bots a cheap `find_path` query so they
//! can route around obstacles instead of walking through them.
//!
//! Coordinates are game-space centimeters, matching `Position.position` xy.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::{Mutex, OnceLock, PoisonError};

use bevy::math::{Vec2, Vec3};

/// Maximum grid extent per axis (cells). Larger zones grow the cell size.
pub const NAV_MAX_CELLS_PER_AXIS: u32 = 1024;
/// Preferred cell resolution in game cm (2m matches the HIM sample spacing).
pub const NAV_PREFERRED_CELL_CM: f32 = 200.0;
/// Bot body radius used to dilate blockers (matches client 0.4m wall ball).
pub const NAV_AGENT_RADIUS_CM: f32 = 40.0;

#[derive(Clone, Debug)]
pub struct NavGrid {
    /// World x (cm) of cell (0,0) corner.
    pub origin_x: f32,
    /// World y (cm) of cell (0,0) corner.
    pub origin_y: f32,
    /// Cell edge length in cm.
    pub cell_cm: f32,
    pub width: u32,
    pub height: u32,
    /// Row-major blocked flags (`y * width + x`), 0 = free, 1 = blocked.
    /// Mark cells through `set_blocked_cell` / `mark_*` so the cached
    /// component labels are invalidated.
    pub blocked: Vec<u8>,
    /// Lazily built connected-component label per cell (0 = blocked), used to
    /// reject unreachable goals without a full A* flood.
    components: OnceLock<Vec<u32>>,
}

/// Snapped start and goal cells of a route query (row-major cell indices).
/// `find_path` returns `None` or a single point purely as a function of this
/// key (the grid is immutable once baked), so callers can cache such results.
pub type NavRouteKey = (u32, u32);

#[derive(Clone, Copy, PartialEq)]
struct Node {
    f: f32,
    g: f32,
    idx: usize,
}

impl Eq for Node {}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse for min-heap; break ties toward larger g (deeper).
        other
            .f
            .total_cmp(&self.f)
            .then_with(|| self.g.total_cmp(&other.g))
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Reusable A* buffers. A cell's `g_score` / `came_from` are only valid when
/// its stamp belongs to the current search, so nothing is cleared between
/// searches (the old per-call allocation was three full-grid arrays).
#[derive(Default)]
struct AStarScratch {
    /// `2 * search` = reached this search, `2 * search + 1` = also closed.
    stamp: Vec<u32>,
    g_score: Vec<f32>,
    came_from: Vec<u32>,
    open: BinaryHeap<Node>,
    search: u32,
}

impl AStarScratch {
    fn begin(&mut self, cells: usize) {
        if self.stamp.len() < cells {
            self.stamp.resize(cells, 0);
            self.g_score.resize(cells, f32::INFINITY);
            self.came_from.resize(cells, u32::MAX);
        }
        self.open.clear();
        // Keep `2 * search + 1` representable; stale stamps must stay below
        // the new search's marks, so wipe them on wrap-around.
        if self.search >= u32::MAX / 2 - 1 {
            self.stamp.fill(0);
            self.search = 0;
        }
        self.search += 1;
    }
}

/// Scratch buffers shared by all grids; one per concurrent search.
static SCRATCH_POOL: Mutex<Vec<AStarScratch>> = Mutex::new(Vec::new());

impl NavGrid {
    pub fn new(origin_x: f32, origin_y: f32, cell_cm: f32, width: u32, height: u32) -> Self {
        Self {
            origin_x,
            origin_y,
            cell_cm: cell_cm.max(50.0),
            width: width.max(1),
            height: height.max(1),
            blocked: vec![0; width.max(1) as usize * height.max(1) as usize],
            components: OnceLock::new(),
        }
    }

    /// Build a grid covering `min..max` world bounds, growing the cell size
    /// if the preferred resolution would exceed the cell cap.
    pub fn covering(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> Self {
        let span_x = (max_x - min_x).max(NAV_PREFERRED_CELL_CM);
        let span_y = (max_y - min_y).max(NAV_PREFERRED_CELL_CM);
        let mut cell_cm = NAV_PREFERRED_CELL_CM;
        while span_x / cell_cm > NAV_MAX_CELLS_PER_AXIS as f32
            || span_y / cell_cm > NAV_MAX_CELLS_PER_AXIS as f32
        {
            cell_cm *= 2.0;
        }
        let width = ((span_x / cell_cm).ceil() as u32).max(1);
        let height = ((span_y / cell_cm).ceil() as u32).max(1);
        Self::new(min_x, min_y, cell_cm, width, height)
    }

    #[inline]
    fn index(&self, cx: u32, cy: u32) -> usize {
        cy as usize * self.width as usize + cx as usize
    }

    pub fn world_to_cell(&self, x: f32, y: f32) -> Option<(u32, u32)> {
        let fx = (x - self.origin_x) / self.cell_cm;
        let fy = (y - self.origin_y) / self.cell_cm;
        if fx < 0.0 || fy < 0.0 {
            return None;
        }
        let cx = fx as u32;
        let cy = fy as u32;
        if cx < self.width && cy < self.height {
            Some((cx, cy))
        } else {
            None
        }
    }

    pub fn cell_to_world_center(&self, cx: u32, cy: u32) -> (f32, f32) {
        (
            self.origin_x + (cx as f32 + 0.5) * self.cell_cm,
            self.origin_y + (cy as f32 + 0.5) * self.cell_cm,
        )
    }

    /// Out of bounds counts as blocked (keeps paths inside baked area).
    pub fn is_blocked_cell(&self, cx: i32, cy: i32) -> bool {
        if cx < 0 || cy < 0 || cx >= self.width as i32 || cy >= self.height as i32 {
            return true;
        }
        self.blocked[cy as usize * self.width as usize + cx as usize] != 0
    }

    pub fn is_blocked_world(&self, x: f32, y: f32) -> bool {
        match self.world_to_cell(x, y) {
            Some((cx, cy)) => self.blocked[self.index(cx, cy)] != 0,
            None => true,
        }
    }

    pub fn set_blocked_cell(&mut self, cx: u32, cy: u32) {
        if cx < self.width && cy < self.height {
            let i = self.index(cx, cy);
            self.blocked[i] = 1;
            self.components.take();
        }
    }

    /// Component labels, built on first use after the last mutation.
    fn components(&self) -> &[u32] {
        self.components.get_or_init(|| self.label_components())
    }

    /// Labels 4-connected free cells. A* moves in 8 directions but never cuts
    /// corners: a diagonal step needs both orthogonal cells free, so it can
    /// always be replaced by two orthogonal steps (and is symmetric). Free
    /// cells are therefore A*-reachable from each other exactly when they
    /// share a 4-connected label.
    fn label_components(&self) -> Vec<u32> {
        let width = self.width as usize;
        let height = self.height as usize;
        let mut labels = vec![0u32; width * height];
        let mut stack = Vec::new();
        let mut next_label = 0u32;

        for seed in 0..width * height {
            if self.blocked[seed] != 0 || labels[seed] != 0 {
                continue;
            }
            next_label += 1;
            labels[seed] = next_label;
            stack.push(seed);

            while let Some(i) = stack.pop() {
                let x = i % width;
                let y = i / width;
                let mut neighbours = [None; 4];
                if x > 0 {
                    neighbours[0] = Some(i - 1);
                }
                if x + 1 < width {
                    neighbours[1] = Some(i + 1);
                }
                if y > 0 {
                    neighbours[2] = Some(i - width);
                }
                if y + 1 < height {
                    neighbours[3] = Some(i + width);
                }
                for n in neighbours.into_iter().flatten() {
                    if self.blocked[n] == 0 && labels[n] == 0 {
                        labels[n] = next_label;
                        stack.push(n);
                    }
                }
            }
        }

        labels
    }

    /// Snapped start and goal cells for a route query, computed exactly as
    /// `find_path` does. `None` means `find_path` returns `None` too.
    pub fn route_key(&self, from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> Option<NavRouteKey> {
        let (sx, sy) = self.nearest_unblocked(from_x, from_y, 5)?;
        let (gx, gy) = self.nearest_unblocked(to_x, to_y, 8)?;
        let (scx, scy) = self.world_to_cell(sx, sy)?;
        let (gcx, gcy) = self.world_to_cell(gx, gy)?;
        Some((self.index(scx, scy) as u32, self.index(gcx, gcy) as u32))
    }

    /// Mark a disc as blocked (world cm). Used for cnst/deco footprints.
    pub fn mark_circle(&mut self, x: f32, y: f32, radius_cm: f32) {
        let r = radius_cm.max(self.cell_cm * 0.5);
        let min_cx = ((x - r - self.origin_x) / self.cell_cm).floor() as i32;
        let max_cx = ((x + r - self.origin_x) / self.cell_cm).ceil() as i32;
        let min_cy = ((y - r - self.origin_y) / self.cell_cm).floor() as i32;
        let max_cy = ((y + r - self.origin_y) / self.cell_cm).ceil() as i32;
        for cy in min_cy..=max_cy {
            for cx in min_cx..=max_cx {
                if cx < 0 || cy < 0 || cx >= self.width as i32 || cy >= self.height as i32 {
                    continue;
                }
                let (wx, wy) = self.cell_to_world_center(cx as u32, cy as u32);
                if Vec2::new(wx - x, wy - y).length() <= r + self.cell_cm * 0.5 {
                    self.set_blocked_cell(cx as u32, cy as u32);
                }
            }
        }
    }

    /// Mark an oriented box as blocked (world cm, yaw radians).
    /// Used for IFO invisible collision walls.
    pub fn mark_obb(&mut self, x: f32, y: f32, half_x: f32, half_y: f32, yaw: f32) {
        let bound = half_x.max(half_y).max(self.cell_cm * 0.5);
        let min_cx = ((x - bound - self.origin_x) / self.cell_cm).floor() as i32;
        let max_cx = ((x + bound - self.origin_x) / self.cell_cm).ceil() as i32;
        let min_cy = ((y - bound - self.origin_y) / self.cell_cm).floor() as i32;
        let max_cy = ((y + bound - self.origin_y) / self.cell_cm).ceil() as i32;
        let (sin, cos) = yaw.sin_cos();
        for cy in min_cy..=max_cy {
            for cx in min_cx..=max_cx {
                if cx < 0 || cy < 0 || cx >= self.width as i32 || cy >= self.height as i32 {
                    continue;
                }
                let (wx, wy) = self.cell_to_world_center(cx as u32, cy as u32);
                let dx = wx - x;
                let dy = wy - y;
                // Rotate into box frame.
                let lx = dx * cos + dy * sin;
                let ly = -dx * sin + dy * cos;
                if lx.abs() <= half_x + self.cell_cm * 0.5
                    && ly.abs() <= half_y + self.cell_cm * 0.5
                {
                    self.set_blocked_cell(cx as u32, cy as u32);
                }
            }
        }
    }

    /// Nearest free cell center to a world point (spiral search).
    pub fn nearest_unblocked(&self, x: f32, y: f32, max_radius_cells: u32) -> Option<(f32, f32)> {
        let (scx, scy) = self.world_to_cell(x, y)?;
        if !self.is_blocked_cell(scx as i32, scy as i32) {
            return Some(self.cell_to_world_center(scx, scy));
        }
        for ring in 1..=max_radius_cells {
            for dy in -(ring as i32)..=(ring as i32) {
                for dx in [-(ring as i32), ring as i32] {
                    let cx = scx as i32 + dx;
                    let cy = scy as i32 + dy;
                    if !self.is_blocked_cell(cx, cy) {
                        return Some(
                            self.cell_to_world_center(cx as u32, cy as u32),
                        );
                    }
                }
            }
            for dx in -(ring as i32) + 1..ring as i32 {
                for dy in [-(ring as i32), ring as i32] {
                    let cx = scx as i32 + dx;
                    let cy = scy as i32 + dy;
                    if !self.is_blocked_cell(cx, cy) {
                        return Some(
                            self.cell_to_world_center(cx as u32, cy as u32),
                        );
                    }
                }
            }
        }
        None
    }

    fn line_blocked(&self, ax: f32, ay: f32, bx: f32, by: f32) -> bool {
        let dist = Vec2::new(bx - ax, by - ay).length();
        let steps = ((dist / (self.cell_cm * 0.5)).ceil() as u32).max(1);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            if self.is_blocked_world(ax + (bx - ax) * t, ay + (by - ay) * t) {
                return true;
            }
        }
        false
    }

    fn smooth_path(&self, path: &[(f32, f32)]) -> Vec<(f32, f32)> {
        if path.len() <= 2 {
            return path.to_vec();
        }
        let mut out = Vec::with_capacity(path.len());
        out.push(path[0]);
        let mut anchor = 0usize;
        while anchor < path.len() - 1 {
            let mut furthest = anchor + 1;
            for i in anchor + 2..path.len() {
                let (ax, ay) = path[anchor];
                let (bx, by) = path[i];
                if self.line_blocked(ax, ay, bx, by) {
                    break;
                }
                furthest = i;
            }
            out.push(path[furthest]);
            anchor = furthest;
        }
        out
    }

    /// A* over cells, returning world-space waypoints (unsmoothed cells then
    /// line-of-sight smoothing). Returns `None` when unreachable.
    pub fn find_path(&self, from_x: f32, from_y: f32, to_x: f32, to_y: f32) -> Option<Vec<(f32, f32)>> {
        let (sx, sy) = self.nearest_unblocked(from_x, from_y, 5)?;
        let (gx, gy) = self.nearest_unblocked(to_x, to_y, 8)?;
        let (scx, scy) = self.world_to_cell(sx, sy)?;
        let (gcx, gcy) = self.world_to_cell(gx, gy)?;
        if scx == gcx && scy == gcy {
            return Some(vec![(gx, gy)]);
        }

        let start_idx = self.index(scx, scy);
        let goal_idx = self.index(gcx, gcy);

        // Different components: A* would exhaust the start's component and
        // return None, so answer immediately. Label 0 (a blocked snap cell,
        // only possible through float rounding) falls through to A*.
        let components = self.components();
        let (start_label, goal_label) = (components[start_idx], components[goal_idx]);
        if start_label != 0 && goal_label != 0 && start_label != goal_label {
            return None;
        }

        let mut scratch = SCRATCH_POOL
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
            .unwrap_or_default();
        let cell_path = self.astar(&mut scratch, start_idx, goal_idx, (gcx, gcy));
        SCRATCH_POOL
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(scratch);

        let raw: Vec<(f32, f32)> = cell_path?
            .iter()
            .map(|&i| {
                let cx = (i % self.width as usize) as u32;
                let cy = (i / self.width as usize) as u32;
                self.cell_to_world_center(cx, cy)
            })
            .collect();
        let mut smoothed = self.smooth_path(&raw);
        // Ensure exact requested endpoints for arrival checks.
        if let Some(first) = smoothed.first_mut() {
            *first = (from_x, from_y);
        }
        if let Some(last) = smoothed.last_mut() {
            *last = (gx, gy);
        }
        Some(smoothed)
    }

    /// A* from `start_idx` to `goal_idx`, returning the cell indices of the
    /// path (start first). Every cell closes at most once, so the search
    /// always terminates within `width * height` expansions; there is no
    /// expansion cap that could abort a reachable search on a max-size grid.
    fn astar(
        &self,
        scratch: &mut AStarScratch,
        start_idx: usize,
        goal_idx: usize,
        (gcx, gcy): (u32, u32),
    ) -> Option<Vec<usize>> {
        let w = self.width as i32;
        let h = self.height as i32;
        let to_idx = |cx: i32, cy: i32| (cy * w + cx) as usize;
        let heuristic = |cx: i32, cy: i32| {
            let dx = (cx - gcx as i32).abs() as f32;
            let dy = (cy - gcy as i32).abs() as f32;
            dx.max(dy) + 0.4142 * dx.min(dy)
        };

        const DIRS: [(i32, i32, f32); 8] = [
            (1, 0, 1.0),
            (-1, 0, 1.0),
            (0, 1, 1.0),
            (0, -1, 1.0),
            (1, 1, 1.4142135),
            (1, -1, 1.4142135),
            (-1, 1, 1.4142135),
            (-1, -1, 1.4142135),
        ];

        scratch.begin(self.width as usize * self.height as usize);
        let reached = scratch.search * 2;
        let closed = reached + 1;

        scratch.stamp[start_idx] = reached;
        scratch.g_score[start_idx] = 0.0;
        scratch.came_from[start_idx] = u32::MAX;
        scratch.open.push(Node {
            f: heuristic(
                (start_idx % self.width as usize) as i32,
                (start_idx / self.width as usize) as i32,
            ),
            g: 0.0,
            idx: start_idx,
        });

        while let Some(Node { g, idx, .. }) = scratch.open.pop() {
            if idx == goal_idx {
                let mut rev = vec![idx];
                let mut cur = idx;
                while cur != start_idx {
                    let prev = scratch.came_from[cur];
                    if prev == u32::MAX {
                        return None;
                    }
                    cur = prev as usize;
                    rev.push(cur);
                }
                rev.reverse();
                return Some(rev);
            }
            // Popped cells were reached this search, so their g is current.
            if scratch.stamp[idx] == closed || g > scratch.g_score[idx] {
                continue;
            }
            scratch.stamp[idx] = closed;
            let cx = (idx % self.width as usize) as i32;
            let cy = (idx / self.width as usize) as i32;
            for (dx, dy, cost) in DIRS {
                let nx = cx + dx;
                let ny = cy + dy;
                if nx < 0 || ny < 0 || nx >= w || ny >= h {
                    continue;
                }
                if self.is_blocked_cell(nx, ny) {
                    continue;
                }
                if dx != 0 && dy != 0
                    && (self.is_blocked_cell(cx + dx, cy)
                        || self.is_blocked_cell(cx, cy + dy))
                {
                    // No corner cutting through walls.
                    continue;
                }
                let nidx = to_idx(nx, ny);
                let ng = g + cost;
                let nstamp = scratch.stamp[nidx];
                // Cells not reached this search have an implicit g of infinity.
                let current_g = if nstamp >= reached {
                    scratch.g_score[nidx]
                } else {
                    f32::INFINITY
                };
                if ng < current_g {
                    scratch.g_score[nidx] = ng;
                    scratch.came_from[nidx] = idx as u32;
                    // A closed cell stays closed (same as the old `closed` flag).
                    if nstamp < reached {
                        scratch.stamp[nidx] = reached;
                    }
                    scratch.open.push(Node {
                        f: ng + heuristic(nx, ny),
                        g: ng,
                        idx: nidx,
                    });
                }
            }
        }
        None
    }

    /// Path as game `Vec3` waypoints, preserving the caller's Z.
    pub fn find_path_vec3(&self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        self.find_path(from.x, from.y, to.x, to.y)
            .map(|pts| pts.into_iter().map(|(x, y)| Vec3::new(x, y, to.z)).collect())
    }

    pub fn blocked_cell_count(&self) -> usize {
        self.blocked.iter().filter(|&&b| b != 0).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_grid() -> NavGrid {
        NavGrid::new(0.0, 0.0, 100.0, 20, 20)
    }

    #[test]
    fn open_field_returns_near_direct_path() {
        let grid = open_grid();
        let path = grid.find_path(100.0, 100.0, 1900.0, 1900.0).expect("path");
        assert!(path.len() <= 3, "smoothed direct path, got {}", path.len());
        let (lx, ly) = *path.last().unwrap();
        assert!((lx - 1900.0).abs() < 150.0 && (ly - 1900.0).abs() < 150.0);
    }

    #[test]
    fn wall_with_gap_routes_through_gap() {
        let mut grid = open_grid();
        // Vertical wall at x=1000 across full height except a gap at y≈1000.
        for cy in 0..20 {
            let (_, wy) = grid.cell_to_world_center(10, cy);
            if (wy - 1050.0).abs() > 150.0 {
                grid.set_blocked_cell(10, cy);
            }
        }
        let path = grid.find_path(100.0, 1000.0, 1900.0, 1000.0).expect("path");
        assert!(path.len() >= 2);
        // Every path SEGMENT must be walkable (smoothing may jump clean over
        // the wall line through the gap, so no waypoint need sit inside it).
        for w in path.windows(2) {
            let (ax, ay) = w[0];
            let (bx, by) = w[1];
            let dist = (bx - ax).hypot(by - ay);
            let steps = ((dist / 50.0).ceil() as u32).max(1);
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                assert!(
                    !grid.is_blocked_world(ax + (bx - ax) * t, ay + (by - ay) * t),
                    "segment crosses wall"
                );
            }
        }
        // Endpoints survive the round trip.
        let (fx, fy) = path[0];
        assert!((fx - 100.0).abs() < 1.0 && (fy - 1000.0).abs() < 1.0);
    }

    #[test]
    fn sealed_goal_snaps_to_nearest_reachable() {
        let mut sealed = NavGrid::new(0.0, 0.0, 100.0, 5, 5);
        for cy in 0..5 {
            for cx in 0..5 {
                if !(cx == 0 && cy == 0) {
                    sealed.set_blocked_cell(cx, cy);
                }
            }
        }
        // Goal deep inside blocked cells snaps out (here, all the way back
        // to the only free cell). Contract: the returned path is always
        // walkable, never a straight walk through walls.
        let path = sealed
            .find_path(50.0, 50.0, 450.0, 450.0)
            .expect("snapped path");
        for (x, y) in &path {
            assert!(!sealed.is_blocked_world(*x, *y));
        }
    }

    #[test]
    fn truly_unreachable_returns_none() {
        // No free cell within snap range (8 rings) of the goal.
        let mut grid = NavGrid::new(0.0, 0.0, 100.0, 21, 21);
        for cy in 0..21 {
            for cx in 0..21 {
                if !(cx == 0 && cy == 0) {
                    grid.set_blocked_cell(cx, cy);
                }
            }
        }
        assert!(grid.find_path(50.0, 50.0, 2050.0, 2050.0).is_none());
    }

    #[test]
    fn out_of_bounds_counts_blocked_and_snaps() {
        let grid = open_grid();
        assert!(grid.is_blocked_world(-10.0, 100.0));
        assert!(grid.is_blocked_world(5000.0, 5000.0));
        let snapped = grid.nearest_unblocked(50.0, 50.0, 3).expect("snap");
        assert!(!grid.is_blocked_world(snapped.0, snapped.1));
    }

    #[test]
    fn mark_circle_blocks_expected_cells() {
        let mut grid = open_grid();
        grid.mark_circle(1000.0, 1000.0, 250.0);
        assert!(grid.is_blocked_world(1000.0, 1000.0));
        assert!(!grid.is_blocked_world(100.0, 100.0));
        assert!(grid.blocked_cell_count() > 4);
    }

    #[test]
    fn diagonal_does_not_cut_corners() {
        // Top-row wall segment; the (0,0)->(1,1) diagonal is illegal because
        // (1,0) is blocked, so the path must detour through row 1.
        let mut grid = NavGrid::new(0.0, 0.0, 100.0, 5, 5);
        grid.set_blocked_cell(1, 0);
        grid.set_blocked_cell(2, 0);
        let path = grid.find_path(50.0, 50.0, 450.0, 50.0).expect("path");
        for (x, y) in &path {
            assert!(!grid.is_blocked_world(*x, *y));
        }
        assert!(
            path.iter().any(|(_, y)| *y >= 150.0),
            "must detour off the walled row"
        );
    }

    #[test]
    fn separate_component_returns_none() {
        // Full vertical wall: both sides are free (no snapping) but disjoint.
        let mut grid = open_grid();
        for cy in 0..20 {
            grid.set_blocked_cell(10, cy);
        }
        assert!(grid.find_path(150.0, 1000.0, 1850.0, 1000.0).is_none());
        // Same side still routes.
        assert!(grid.find_path(150.0, 150.0, 850.0, 1850.0).is_some());
    }

    #[test]
    fn diagonal_only_gap_is_not_a_connection() {
        // Two free regions touching only diagonally: corner cutting is
        // forbidden, so they are separate components for A* too.
        let mut grid = NavGrid::new(0.0, 0.0, 100.0, 4, 4);
        for cy in 0..4 {
            for cx in 0..4 {
                let left_top = cx <= 1 && cy <= 1;
                let right_bottom = cx >= 2 && cy >= 2;
                if !left_top && !right_bottom {
                    grid.set_blocked_cell(cx, cy);
                }
            }
        }
        assert!(grid.find_path(50.0, 50.0, 350.0, 350.0).is_none());
    }

    #[test]
    fn reused_scratch_gives_identical_paths() {
        let mut walled = open_grid();
        for cy in 0..20 {
            if !(9..=11).contains(&cy) {
                walled.set_blocked_cell(10, cy);
            }
        }
        let small = NavGrid::new(0.0, 0.0, 100.0, 5, 5);
        let first = walled.find_path(100.0, 300.0, 1900.0, 1700.0).expect("path");
        // Interleave searches on a smaller grid so stale stamps are present.
        small.find_path(50.0, 50.0, 450.0, 450.0).expect("small path");
        let second = walled.find_path(100.0, 300.0, 1900.0, 1700.0).expect("path");
        assert_eq!(first, second);
    }

    #[test]
    fn mutation_invalidates_component_labels() {
        let mut grid = open_grid();
        assert!(grid.find_path(150.0, 1000.0, 1850.0, 1000.0).is_some());
        for cy in 0..20 {
            grid.set_blocked_cell(10, cy);
        }
        assert!(grid.find_path(150.0, 1000.0, 1850.0, 1000.0).is_none());
    }

    #[test]
    fn route_key_matches_snapping() {
        let grid = open_grid();
        let key = grid.route_key(150.0, 150.0, 1850.0, 1850.0).expect("key");
        assert_eq!(key, (grid.index(1, 1) as u32, grid.index(18, 18) as u32));
        assert!(grid.route_key(-500.0, 150.0, 1850.0, 1850.0).is_none());
    }
}
