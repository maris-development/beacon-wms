//! Split the cold path of a layer and compare it against raw disk I/O.
//!
//! Usage: cargo run --release --example indexbuild -- <layer.parquet>

use arrow::array::RecordBatch;
use rust_backend::boundingbox::BoundingBox;
use rust_backend::cache_engine;
use rust_backend::data_utils;
use rust_backend::map_drawing::{LATITUDE_COLUMN, LONGITUDE_COLUMN, VALUE_COLUMN};
use rust_backend::point_index::LayerPoints;

use std::fs::File;
use std::io::{Read, Write};
use std::time::Instant;

const CRS: &str = "EPSG:3857";

fn main() {
    let path = std::env::args().nth(1).expect("give a layer parquet path");

    let bbox = BoundingBox::new(-1.0, -1.0, 1.0, 1.0, CRS);
    let bounds = bbox.get_max_bounds();
    let world = (
        bounds.get_min_x(),
        bounds.get_min_y(),
        bounds.get_max_x(),
        bounds.get_max_y(),
    );

    // Phase A: read the three columns out of parquet.
    let start = Instant::now();
    let reader = data_utils::parquet_reader(
        &path,
        File::open(&path).unwrap(),
        Some(&[LONGITUDE_COLUMN, LATITUDE_COLUMN, VALUE_COLUMN]),
    )
    .unwrap();

    let raw: Vec<RecordBatch> = reader.map(|b| b.unwrap()).collect();
    let decode = start.elapsed().as_secs_f64();
    let rows: usize = raw.iter().map(|b| b.num_rows()).sum();

    // Phase B: reproject every batch to the target CRS.
    let start = Instant::now();
    let batches: Vec<RecordBatch> = raw
        .into_iter()
        .map(|b| cache_engine::reproject_batch("EPSG:4326", CRS, b).unwrap())
        .collect();
    let reproject = start.elapsed().as_secs_f64();

    // Phase C: sort by Morton code and build every level.
    let start = Instant::now();
    let points = LayerPoints::build(&batches, world).unwrap();
    let build = start.elapsed().as_secs_f64();

    drop(batches);

    let bytes = points.bytes();

    println!("rows            {}", rows);
    println!("index size      {:.0} MB", bytes as f64 / 1_048_576.0);
    println!();
    println!("A parquet read  {:7.2} s", decode);
    println!("B reproject     {:7.2} s", reproject);
    println!("C sort + levels {:7.2} s", build);
    println!("  cold total    {:7.2} s", decode + reproject + build);
    println!();

    // Raw disk I/O of the same volume, as the floor for a prebuilt index on disk.
    let blob = vec![0u8; bytes];
    let scratch = std::env::temp_dir().join("beacon_index_blob.bin");

    let start = Instant::now();
    let mut file = File::create(&scratch).unwrap();
    file.write_all(&blob).unwrap();
    file.sync_all().unwrap();
    let write = start.elapsed().as_secs_f64();

    drop(blob);

    let mut back = vec![0u8; bytes];
    let start = Instant::now();
    File::open(&scratch).unwrap().read_exact(&mut back).unwrap();
    let read_warm = start.elapsed().as_secs_f64();

    println!("write {:.0} MB   {:7.2} s", bytes as f64 / 1_048_576.0, write);
    println!("read back       {:7.2} s  ({:.0} MB/s, page cache warm)", read_warm, bytes as f64 / 1_048_576.0 / read_warm);

    std::fs::remove_file(&scratch).ok();
}
