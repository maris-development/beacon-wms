//! Render a screen worth of WMS tiles and report the cost per tile.
//!
//! Usage: cargo run --release --example tilebench -- <layer.parquet> [lon] [lat] [zooms]
//! Example: cargo run --release --example tilebench -- ../layers/foo.parquet 4.3 52.3 4,6,10,14

use rust_backend::boundingbox::BoundingBox;
use rust_backend::color_maps::ColorMap;
use rust_backend::image_utils;
use rust_backend::map_drawing;
use rust_backend::request_profiling::RequestProfiling;

use std::cell::RefCell;
use std::fs::File;
use std::time::Instant;

const CRS: &str = "EPSG:3857";
const TILE: u32 = 256;
const WORLD: f64 = 20037508.342789244;

/// Tiles that a 3840x2160 viewport asks for, plus the Leaflet keep buffer.
const COLS: i64 = 15;
const ROWS: i64 = 9;

thread_local! {
    static DUMP_NAME: RefCell<String> = RefCell::new(String::new());
}

/// Web Mercator bounds of one slippy tile.
fn tile_bbox(z: u32, x: i64, y: i64) -> BoundingBox {
    let span = (WORLD * 2.0) / (1i64 << z) as f64;

    BoundingBox::new(
        -WORLD + x as f64 * span,
        WORLD - (y + 1) as f64 * span,
        -WORLD + (x + 1) as f64 * span,
        WORLD - y as f64 * span,
        CRS,
    )
}

/// Slippy tile that holds a WGS84 coordinate.
fn tile_of(z: u32, lon: f64, lat: f64) -> (i64, i64) {
    let n = (1i64 << z) as f64;
    let x = (lon + 180.0) / 360.0 * n;
    let sin = lat.to_radians().sin();
    let y = (0.5 - ((1.0 + sin) / (1.0 - sin)).ln() / (4.0 * std::f64::consts::PI)) * n;

    (x as i64, y as i64)
}

fn render_one(path: &str, bbox: BoundingBox, cmap: &ColorMap) -> (f64, f64, map_drawing::DrawStats) {
    let file = File::open(path).unwrap();
    let mut image = image_utils::create_rgba_image(TILE, TILE);
    let mut profiling = RequestProfiling::new();

    let start = Instant::now();

    let stats = map_drawing::get_map(
        &mut image,
        bbox,
        cmap.clone(),
        CRS,
        path.to_string(),
        file,
        0,
        "circle",
        &mut profiling,
    )
    .unwrap();

    let draw_ms = start.elapsed().as_secs_f64() * 1000.0;

    let encode_start = Instant::now();
    let mut png = Vec::new();
    image_utils::rgba_image_to_png(&image, &mut png).unwrap();
    let encode_ms = encode_start.elapsed().as_secs_f64() * 1000.0;

    // TILEBENCH_DUMP names a directory. Every tile lands there for a diff between builds.
    if let Ok(dir) = std::env::var("TILEBENCH_DUMP") {
        std::fs::create_dir_all(&dir).unwrap();
        image.save(format!("{}/{}.png", dir, DUMP_NAME.with(|n| n.borrow().clone()))).unwrap();
    }

    (draw_ms, encode_ms, stats)
}

fn main() {
    let mut args = std::env::args().skip(1);

    let path = args.next().expect("give a layer parquet path");
    let lon: f64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(4.3);
    let lat: f64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(52.3);
    let zooms: Vec<u32> = args
        .next()
        .unwrap_or_else(|| String::from("4,6,10,14"))
        .split(',')
        .map(|s| s.trim().parse().unwrap())
        .collect();

    let cmap = ColorMap::get_named("thermal", -10.0, 100.0, None).expect("colormap thermal");

    println!("layer  {}", path);
    println!("centre {}, {}", lon, lat);
    println!("screen {}x{} tiles of {} px\n", COLS, ROWS, TILE);

    println!(
        "{:>4}  {:>9}  {:>9}  {:>9}  {:>8}  {:>12}  {:>11}",
        "zoom", "cold ms", "screen ms", "draw/tile", "png/tile", "scanned/tile", "drawn/tile"
    );

    for z in zooms {
        let (cx, cy) = tile_of(z, lon, lat);

        // The first tile builds the reprojection cache. Report it apart from the rest.
        DUMP_NAME.with(|n| *n.borrow_mut() = format!("z{}_cold", z));
        let (cold, _, _) = render_one(&path, tile_bbox(z, cx, cy), &cmap);

        let mut draw_total = 0.0;
        let mut encode_total = 0.0;
        let mut drawn_total = 0usize;
        let mut scanned_total = 0usize;
        let mut tiles = 0usize;

        for row in 0..ROWS {
            for col in 0..COLS {
                let x = cx + col - COLS / 2;
                let y = cy + row - ROWS / 2;

                if y < 0 || y >= (1i64 << z) {
                    continue;
                }

                DUMP_NAME.with(|n| *n.borrow_mut() = format!("z{}_{}_{}", z, x, y));
                let (draw_ms, encode_ms, stats) = render_one(&path, tile_bbox(z, x, y), &cmap);

                draw_total += draw_ms;
                encode_total += encode_ms;
                drawn_total += stats.drawn;
                scanned_total += stats.scanned;
                tiles += 1;
            }
        }

        let tiles_f = tiles.max(1) as f64;

        println!(
            "{:>4}  {:>9.1}  {:>9.1}  {:>9.2}  {:>8.2}  {:>12}  {:>11}",
            z,
            cold,
            draw_total + encode_total,
            draw_total / tiles_f,
            encode_total / tiles_f,
            scanned_total / tiles.max(1),
            drawn_total / tiles.max(1)
        );
    }

    println!(
        "
point index cache: {:.0} MB",
        rust_backend::map_drawing::POINT_INDEX_CACHE.bytes() as f64 / 1_048_576.0
    );
}
