//! Renders the server's baked zone walkability grids (`rose_data::NavGrid`)
//! to north-up PNG maps, one pixel per cell, so the bake can be checked by
//! eye before testing bots in game. Loads the zones exactly as the server
//! does (`rose_data_irose::get_zone_database`), which also times the bake.
//!
//! Colors: walkable ground is shaded by floor height (green-grey, lighter =
//! higher); cyan = walkable but stranded (no start / revive / NPC / spawn
//! point leads there); red = wall, orange = slope, yellow = ledge, purple =
//! under objects with no reachable floor, blue = deep water, magenta = warp
//! gate, pink = body clearance, black = outside the zone. Markers: white
//! cross = start, light blue cross = revive, white dot = NPC, green dot =
//! monster spawn.

use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use anyhow::Context;
use clap::{Arg, Command};

use rose_data::{
    NavGrid, ZoneData, ZoneDatabase, ZoneId, NAV_BLOCK_CLEARANCE, NAV_BLOCK_LEDGE, NAV_BLOCK_NO_FLOOR,
    NAV_BLOCK_SLOPE, NAV_BLOCK_VOID, NAV_BLOCK_WALL, NAV_BLOCK_WARP, NAV_BLOCK_WATER,
};
use rose_file_readers::{HostFilesystemDevice, VfsIndex, VirtualFilesystem, VirtualFilesystemDevice};

type Rgb = [u8; 3];

/// Blocked reasons in drawing priority order.
const BLOCK_COLORS: [(u8, Rgb); 8] = [
    (NAV_BLOCK_VOID, [16, 16, 16]),
    (NAV_BLOCK_WARP, [220, 60, 220]),
    (NAV_BLOCK_WALL, [205, 40, 40]),
    (NAV_BLOCK_WATER, [40, 90, 210]),
    (NAV_BLOCK_LEDGE, [235, 205, 40]),
    (NAV_BLOCK_SLOPE, [215, 120, 40]),
    (NAV_BLOCK_NO_FLOOR, [125, 60, 170]),
    (NAV_BLOCK_CLEARANCE, [235, 160, 160]),
];
const STRANDED_COLOR: Rgb = [70, 210, 225];

fn main() -> anyhow::Result<()> {
    let matches = Command::new("rose-nav-map")
        .about("Render the server's baked zone walkability grids to PNG maps")
        .arg(
            Arg::new("data-idx")
                .long("data-idx")
                .help("Path to irose data.idx")
                .takes_value(true),
        )
        .arg(
            Arg::new("data-path")
                .long("data-path")
                .help("Optional path to extracted data, any files here override ones in data.idx")
                .takes_value(true),
        )
        .arg(
            Arg::new("zone")
                .long("zone")
                .help("Zone id to render (repeatable); default: every zone with a grid")
                .takes_value(true)
                .multiple_occurrences(true),
        )
        .arg(
            Arg::new("output")
                .long("output")
                .help("Output directory, defaults to 'nav-maps'")
                .takes_value(true),
        )
        .arg(
            Arg::new("probe")
                .long("probe")
                .help(
                    "Describe one map pixel instead of rendering: <zone>,<px>,<py>[,<radius_m>] \
                     with px/py as in that zone's PNG. Prints the cell and every object part \
                     within the radius (default 5 m).",
                )
                .takes_value(true),
        )
        .get_matches();

    simplelog::SimpleLogger::init(log::LevelFilter::Info, simplelog::Config::default())?;

    let mut data_idx_path = matches.value_of("data-idx").map(PathBuf::from);
    let data_extracted_path = matches.value_of("data-path").map(PathBuf::from);
    if data_idx_path.is_none() && data_extracted_path.is_none() {
        if Path::new("data.idx").exists() {
            data_idx_path = Some(PathBuf::from("data.idx"));
        } else {
            anyhow::bail!("Specify --data-idx or --data-path, or run next to data.idx");
        }
    }

    let mut devices: Vec<Box<dyn VirtualFilesystemDevice + Send + Sync>> = Vec::new();
    if let Some(path) = data_extracted_path {
        devices.push(Box::new(HostFilesystemDevice::new(path)));
    }
    if let Some(path) = data_idx_path {
        let index = VfsIndex::load(&path)
            .with_context(|| format!("Failed to load VFS index {}", path.display()))?;
        devices.push(Box::new(index));
        devices.push(Box::new(HostFilesystemDevice::new(
            path.parent().map(Path::to_path_buf).unwrap_or_default(),
        )));
    }
    let vfs = VirtualFilesystem::new(devices);

    let strings = rose_data_irose::get_string_database(&vfs, 1)?;
    let zones = rose_data_irose::get_zone_database(&vfs, strings)?;

    if let Some(probe) = matches.value_of("probe") {
        return run_probe(&vfs, &zones, probe);
    }

    let wanted: Option<Vec<u16>> = matches
        .values_of("zone")
        .map(|values| values.filter_map(|value| value.parse().ok()).collect());
    let output = PathBuf::from(matches.value_of("output").unwrap_or("nav-maps"));
    std::fs::create_dir_all(&output)?;

    let mut summary = BufWriter::new(File::create(output.join("summary.txt"))?);
    writeln!(
        summary,
        "zone\tname\tcells\tcell_cm\tfree%\tstranded%\twall%\tslope%\tledge%\tno_floor%\twater%\twarp%\tvoid%"
    )?;
    for zone in zones.iter() {
        if wanted
            .as_ref()
            .is_some_and(|wanted| !wanted.contains(&zone.id.get()))
        {
            continue;
        }
        let Some(grid) = zone.nav_grid.as_ref() else {
            log::info!("Zone {} {}: no walkability grid", zone.id.get(), zone.name);
            continue;
        };
        let path = output.join(format!("zone_{:03}.png", zone.id.get()));
        let stranded = render(zone, grid, &path)?;
        let cells = grid.blocked.len().max(1);
        let percent = |count: usize| 100.0 * count as f32 / cells as f32;
        let flag_percent = |flag: u8| percent(grid.flag_cell_count(flag));
        let line = format!(
            "{}\t{}\t{}x{}\t{:.0}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}",
            zone.id.get(),
            zone.name,
            grid.width,
            grid.height,
            grid.cell_cm,
            percent(cells - grid.blocked_cell_count()),
            percent(stranded),
            flag_percent(NAV_BLOCK_WALL),
            flag_percent(NAV_BLOCK_SLOPE),
            flag_percent(NAV_BLOCK_LEDGE),
            flag_percent(NAV_BLOCK_NO_FLOOR),
            flag_percent(NAV_BLOCK_WATER),
            flag_percent(NAV_BLOCK_WARP),
            flag_percent(NAV_BLOCK_VOID),
        );
        writeln!(summary, "{}", line)?;
        log::info!("{} -> {}", line, path.display());
    }
    summary.flush()?;
    Ok(())
}

/// `--probe <zone>,<px>,<py>[,<radius_m>]`: print one cell and the objects
/// around it.
fn run_probe(vfs: &VirtualFilesystem, zones: &ZoneDatabase, probe: &str) -> anyhow::Result<()> {
    let values: Vec<f32> = probe
        .split(',')
        .map(|value| value.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .context("--probe expects <zone>,<px>,<py>[,<radius_m>]")?;
    let [zone_id, px, py, rest @ ..] = values.as_slice() else {
        anyhow::bail!("--probe expects <zone>,<px>,<py>[,<radius_m>]");
    };
    let radius_cm = rest.first().copied().unwrap_or(5.0) * 100.0;
    let zone = ZoneId::new(*zone_id as u16)
        .and_then(|id| zones.get_zone(id))
        .with_context(|| format!("zone {} not loaded", zone_id))?;
    let grid = zone.nav_grid.as_ref().context("zone has no walkability grid")?;
    let (cx, cy) = (*px as u32, grid.height.saturating_sub(1 + *py as u32));
    let (x, y) = grid.cell_to_world_center(cx, cy);
    let label = grid.component_at_world(x, y);
    println!(
        "zone {} {}: pixel ({}, {}) = cell ({}, {}) = world ({:.0}, {:.0}); flags {}; floor {:?}; component {} (anchored: {})",
        zone.id.get(),
        zone.name,
        px,
        py,
        cx,
        cy,
        x,
        y,
        flag_names(grid.flags_world(x, y)),
        grid.floor_height_world(x, y),
        label,
        grid.is_anchored_component(label),
    );
    for line in
        rose_data_irose::describe_nav_objects_near(vfs, zone.id.get() as usize, x, y, radius_cm)?
    {
        println!("  {}", line);
    }
    Ok(())
}

fn flag_names(flags: u8) -> String {
    if flags == 0 {
        return "walkable".into();
    }
    [
        (NAV_BLOCK_WALL, "wall"),
        (NAV_BLOCK_SLOPE, "slope"),
        (NAV_BLOCK_LEDGE, "ledge"),
        (NAV_BLOCK_NO_FLOOR, "no-floor"),
        (NAV_BLOCK_VOID, "void"),
        (NAV_BLOCK_WATER, "water"),
        (NAV_BLOCK_WARP, "warp"),
        (NAV_BLOCK_CLEARANCE, "clearance"),
    ]
    .iter()
    .filter(|(flag, _)| flags & flag != 0)
    .map(|(_, name)| *name)
    .collect::<Vec<_>>()
    .join(",")
}

/// Write one zone's map; returns the number of stranded free cells.
fn render(zone: &ZoneData, grid: &NavGrid, path: &Path) -> anyhow::Result<usize> {
    let (width, height) = (grid.width as usize, grid.height as usize);
    let center = |cx: usize, cy: usize| grid.cell_to_world_center(cx as u32, cy as u32);
    let floor_at = |cx: usize, cy: usize| {
        let (x, y) = center(cx, cy);
        grid.floor_height_world(x, y)
    };

    // Height range of walkable floors, for shading.
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for cy in 0..height {
        for cx in 0..width {
            if grid.blocked[cy * width + cx] == 0 {
                if let Some(z) = floor_at(cx, cy) {
                    low = low.min(z);
                    high = high.max(z);
                }
            }
        }
    }
    let range = (high - low).max(1.0);

    let mut stranded = 0;
    // PNG rows run north to south; grid row 0 is the south edge.
    let mut pixels = vec![0u8; width * height * 3];
    for cy in 0..height {
        for cx in 0..width {
            let flags = grid.blocked[cy * width + cx];
            let color = if flags != 0 {
                BLOCK_COLORS
                    .iter()
                    .find(|(flag, _)| flags & flag != 0)
                    .map_or([0, 0, 0], |(_, color)| *color)
            } else {
                let (x, y) = center(cx, cy);
                if !grid.is_anchored_component(grid.component_at_world(x, y)) {
                    stranded += 1;
                    STRANDED_COLOR
                } else {
                    let z = floor_at(cx, cy).unwrap_or(low);
                    // Height tint plus a north-west light for relief.
                    let west = if cx > 0 { floor_at(cx - 1, cy) } else { None }.unwrap_or(z);
                    let north = floor_at(cx, (cy + 1).min(height - 1)).unwrap_or(z);
                    let relief = ((z - west) + (z - north)) / grid.cell_cm;
                    let level = 70.0 + 140.0 * (z - low) / range + relief.clamp(-1.0, 1.0) * 25.0;
                    let level = level.clamp(30.0, 245.0);
                    [(level * 0.82) as u8, level as u8, (level * 0.74) as u8]
                }
            };
            let row = height - 1 - cy;
            pixels[(row * width + cx) * 3..][..3].copy_from_slice(&color);
        }
    }

    let mut mark = |x: f32, y: f32, radius: i32, cross: bool, color: Rgb| {
        let Some((cx, cy)) = grid.world_to_cell(x, y) else {
            return;
        };
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if cross && dx != 0 && dy != 0 {
                    continue;
                }
                let (px, py) = (cx as i32 + dx, cy as i32 + dy);
                if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                    continue;
                }
                let row = height - 1 - py as usize;
                pixels[(row * width + px as usize) * 3..][..3].copy_from_slice(&color);
            }
        }
    };
    let marker = ((300.0 / grid.cell_cm) as i32).max(1);
    for spawn in zone.monster_spawns.iter() {
        mark(spawn.position.x, spawn.position.y, marker, false, [40, 240, 60]);
    }
    for npc in zone.npcs.iter() {
        mark(npc.position.x, npc.position.y, marker, false, [255, 255, 255]);
    }
    for revive in zone.revive_positions.iter() {
        mark(revive.x, revive.y, marker * 2, true, [150, 200, 255]);
    }
    mark(
        zone.start_position.x,
        zone.start_position.y,
        marker * 3,
        true,
        [255, 255, 255],
    );

    let file = BufWriter::new(File::create(path)?);
    let mut encoder = png::Encoder::new(file, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&pixels)?;
    Ok(stranded)
}
