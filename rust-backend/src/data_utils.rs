use crate::errors::MapError;
use arrow::array::RecordBatch;
use arrow::error::ArrowError;
use log;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ProjectionMask;
use std::fs::File;

pub const PARQUET_BATCH_SIZE: usize = 128 * 1024; //128k rows per batch

/// Read only the parquet footer to determine how many record batches the file will produce.
/// No data pages are read, so this is very cheap (~1ms).
///
/// Batches cross row group boundaries, so the row count alone gives the answer. A column
/// projection does not change it either, so cache indices stay aligned.
pub fn get_parquet_batch_count(layer_filepath: &str, file: File) -> Result<usize, MapError> {
    
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
        MapError::Error(format!("Could not read parquet metadata for file {}: {}", layer_filepath, e))
    })?;

    let total_rows = builder.metadata().file_metadata().num_rows() as usize;

    Ok((total_rows + PARQUET_BATCH_SIZE - 1) / PARQUET_BATCH_SIZE)
}

/// Open a record batch reader over a layer file.
///
/// `columns` limits the read to those column names. A name that the file does not hold is
/// skipped. `None` reads every column. Drawing needs three columns, so a projection saves
/// most of the decode work. GetFeatureInfo puts every column in the response, so it passes
/// `None`.
pub fn parquet_reader(
    layer_filepath: &str,
    file: File,
    columns: Option<&[&str]>,
) -> Result<Box<dyn Iterator<Item = Result<RecordBatch, ArrowError>>>, MapError> {

    let builder = match ParquetRecordBatchReaderBuilder::try_new(file) {
        Ok(b) => b,
        Err(e) => {
            log::error!("1. Could not create parquet reader for layer file: {} \n{:?}", layer_filepath, e);
            return Err(MapError::Error(format!(
                "1. Could not create parquet reader for layer file: {}",
                layer_filepath
            )));
        }
    };

    // ProjectionMask::columns matches a name by prefix, so "value" would also take
    // "value_qc". Match the leaf names exactly instead.
    let builder = match columns {
        Some(names) => {
            let descriptor = builder.parquet_schema();

            let indices: Vec<usize> = (0..descriptor.num_columns())
                .filter(|i| names.contains(&descriptor.column(*i).name()))
                .collect();

            let mask = ProjectionMask::leaves(descriptor, indices);

            builder.with_projection(mask)
        }
        None => builder,
    };

    let reader = match builder.with_batch_size(PARQUET_BATCH_SIZE).build() {
        Ok(r) => r,
        Err(e) => {
            log::error!("2. Could not create parquet reader for layer file: {} \n{:?}", layer_filepath, e);
            return Err(MapError::Error(format!(
                "2. Could not create parquet reader for layer file: {}",
                layer_filepath
            )));
        }
    };

    let reader: Box<dyn Iterator<Item = Result<RecordBatch, ArrowError>>> =
        Box::new(reader.into_iter());

    Ok(reader)
}
