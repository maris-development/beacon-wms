use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ProjectionMask;
use std::fs::File;
use std::time::Instant;

const BATCH: usize = 128 * 1024;

fn main() {
    let path = std::env::args().nth(1).unwrap();

    let file = File::open(&path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let meta = builder.metadata().clone();
    let schema = builder.parquet_schema().clone();

    println!("rows: {}", meta.file_metadata().num_rows());
    println!("row groups: {}", meta.num_row_groups());
    println!("columns:");
    let mut total = 0u64;
    for rg in meta.row_groups() {
        for c in rg.columns() {
            total += c.compressed_size() as u64;
        }
    }
    for i in 0..meta.row_groups()[0].columns().len() {
        let mut comp = 0i64;
        let mut uncomp = 0i64;
        let name = meta.row_groups()[0].column(i).column_path().string();
        for rg in meta.row_groups() {
            comp += rg.column(i).compressed_size();
            uncomp += rg.column(i).uncompressed_size();
        }
        println!(
            "  {:<12} compressed {:>10} B  uncompressed {:>10} B  ({:>4.1}% of file)",
            name, comp, uncomp, comp as f64 / total as f64 * 100.0
        );
    }

    let wanted = ["longitude", "latitude", "value"];
    let indices: Vec<usize> = (0..schema.columns().len())
        .filter(|i| wanted.contains(&schema.column(*i).name()))
        .collect();
    println!("projection indices for {:?}: {:?}", wanted, indices);

    for label in ["all columns", "projected"] {
        // Three runs, report the best, so the page cache is warm.
        let mut best = f64::MAX;
        for _ in 0..3 {
            let file = File::open(&path).unwrap();
            let b = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
            let b = if label == "projected" {
                b.with_projection(ProjectionMask::roots(&schema, indices.clone()))
            } else {
                b
            };
            let reader = b.with_batch_size(BATCH).build().unwrap();

            let start = Instant::now();
            let mut rows = 0usize;
            for batch in reader {
                rows += batch.unwrap().num_rows();
            }
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            if ms < best {
                best = ms;
            }
            assert!(rows > 0);
        }
        println!("{:<12} decode {:>8.1} ms", label, best);
    }
}
