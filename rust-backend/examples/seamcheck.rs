//! Compare a mosaic of adjacent tiles against one render of the same extent.
//!
//! Usage: cargo run --release --example seamcheck -- <layer.parquet> <lon> <lat> <zoom> [cols] [rows]

use image::RgbaImage;
use rust_backend::boundingbox::BoundingBox;
use rust_backend::color_maps::ColorMap;
use rust_backend::image_utils;
use rust_backend::map_drawing;
use rust_backend::request_profiling::RequestProfiling;

use std::fs::File;

const CRS: &str = "EPSG:3857";
const TILE: u32 = 256;
const WORLD: f64 = 20037508.342789244;

fn bbox_of(z: u32, x0: i64, y0: i64, cols: i64, rows: i64) -> BoundingBox {
    let span = (WORLD * 2.0) / (1i64 << z) as f64;

    BoundingBox::new(
        -WORLD + x0 as f64 * span,
        WORLD - (y0 + rows) as f64 * span,
        -WORLD + (x0 + cols) as f64 * span,
        WORLD - y0 as f64 * span,
        CRS,
    )
}

fn tile_of(z: u32, lon: f64, lat: f64) -> (i64, i64) {
    let n = (1i64 << z) as f64;
    let x = (lon + 180.0) / 360.0 * n;
    let sin = lat.to_radians().sin();
    let y = (0.5 - ((1.0 + sin) / (1.0 - sin)).ln() / (4.0 * std::f64::consts::PI)) * n;

    (x as i64, y as i64)
}

fn render(path: &str, bbox: BoundingBox, w: u32, h: u32, cmap: &ColorMap) -> RgbaImage {
    let file = File::open(path).unwrap();
    let mut image = image_utils::create_rgba_image(w, h);
    let mut profiling = RequestProfiling::new();

    map_drawing::get_map(
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

    image
}

fn main() {
    let mut args = std::env::args().skip(1);

    let path = args.next().expect("layer parquet path");
    let lon: f64 = args.next().unwrap().parse().unwrap();
    let lat: f64 = args.next().unwrap().parse().unwrap();
    let z: u32 = args.next().unwrap().parse().unwrap();
    let cols: i64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(4);
    let rows: i64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(4);

    let cmap = ColorMap::get_named("thermal", -10.0, 100.0, None).expect("colormap");

    let (cx, cy) = tile_of(z, lon, lat);
    let (x0, y0) = (cx - cols / 2, cy - rows / 2);

    let (w, h) = (cols as u32 * TILE, rows as u32 * TILE);

    let whole = render(&path, bbox_of(z, x0, y0, cols, rows), w, h, &cmap);

    let mut mosaic = image_utils::create_rgba_image(w, h);

    for row in 0..rows {
        for col in 0..cols {
            let tile = render(&path, bbox_of(z, x0 + col, y0 + row, 1, 1), TILE, TILE, &cmap);

            for ty in 0..TILE {
                for tx in 0..TILE {
                    let p = *tile.get_pixel(tx, ty);
                    mosaic.put_pixel(col as u32 * TILE + tx, row as u32 * TILE + ty, p);
                }
            }
        }
    }

    // Difference per distance to the nearest tile border.
    let mut by_distance = vec![(0usize, 0usize); 24];
    let mut diff_img = image_utils::create_rgba_image(w, h);

    for y in 0..h {
        for x in 0..w {
            let a = whole.get_pixel(x, y);
            let b = mosaic.get_pixel(x, y);

            let dx = (x % TILE).min(TILE - 1 - (x % TILE));
            let dy = (y % TILE).min(TILE - 1 - (y % TILE));
            let d = (dx.min(dy) as usize).min(by_distance.len() - 1);

            by_distance[d].1 += 1;

            if a != b {
                by_distance[d].0 += 1;
                diff_img.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
    }

    whole.save("../logs/seam_whole.png").unwrap();
    mosaic.save("../logs/seam_mosaic.png").unwrap();
    diff_img.save("../logs/seam_diff.png").unwrap();

    println!("zoom {z}  tiles {cols}x{rows}  origin {x0},{y0}");
    println!("{:>4}  {:>10}  {:>10}  {:>7}", "dist", "different", "pixels", "share");

    for (d, (bad, total)) in by_distance.iter().enumerate() {
        if *total == 0 {
            continue;
        }

        println!(
            "{:>4}  {:>10}  {:>10}  {:>6.2}%",
            d,
            bad,
            total,
            100.0 * *bad as f64 / *total as f64
        );
    }
}
