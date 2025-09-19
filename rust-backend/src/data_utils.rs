use crate::errors::MapError;
use arrow::array::RecordBatch;
use arrow::error::ArrowError;
use log;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use std::fs::File;

pub const PARQUET_BATCH_SIZE: usize = 128 * 1024; //128k rows per batch

pub fn open_parquet_reader(layer: &str, layer_filepath: &str) -> Result<Box<dyn Iterator<Item = Result<RecordBatch, ArrowError>>>, MapError> {
    if !std::path::Path::new(&layer_filepath).exists() {
        log::error!("Layer file does not exist: {}", layer_filepath);
        return Err(MapError::Error(format!(
            "Layer file does not exist: {}",
            layer
        )));
    }

    // read layer file (using arrow)
    let file = match File::open(&layer_filepath) {
        Ok(f) => f,
        Err(e) => {
            log::error!("Could not open layer file: {} \n{:?}", layer_filepath, e);
            return Err(MapError::Error(format!(
                "Could not open layer file: {}",
                layer
            )));
        }
    };
    

    let builder = ParquetRecordBatchReaderBuilder::try_new(file);
    
    let reader = match builder {
        Ok(b) => b
            .with_batch_size(PARQUET_BATCH_SIZE)
            .build(),
        Err(e) => {
            log::error!("Could not create parquet reader for layer file: {} \n{:?}", layer_filepath, e);
            return Err(MapError::Error(format!(
                "Could not create parquet reader for layer file: {}",
                layer
            )));
        }
    };

    let reader = match reader {
        Ok(r) => r,
        Err(e) => {
            log::error!("Could not create parquet reader for layer file: {} \n{:?}", layer_filepath, e);
            return Err(MapError::Error(format!(
                "Could not create parquet reader for layer file: {}",
                layer
            )));
        }
    };


    let reader: Box<dyn Iterator<Item = Result<RecordBatch, ArrowError>>> =
        Box::new(reader.into_iter());


    Ok(reader)
}

