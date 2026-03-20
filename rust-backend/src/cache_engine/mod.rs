use arrow::{array::{Float64Array, RecordBatch}, datatypes::{DataType, Field}};
use lru::LruCache;
use std::{
    num::NonZeroUsize,
    sync::{Arc, RwLock},
};

use crate::{
    errors::MapError,
    map_drawing::{LATITUDE_COLUMN, LONGITUDE_COLUMN},
    misc,
};

pub const LRU_CACHE_SIZE: usize = 50000;

/// Projection Engine
///
/// This struct is used to apply projections to record batches and store the result
/// This is synchronized and can safely be used from multiple threads
pub struct ReprojectedDatasetCacheEngine {
    inner: Arc<RwLock<InnerReprojectedDatasetCacheEngine>>,
}

impl ReprojectedDatasetCacheEngine {
    pub fn new() -> Self {
        ReprojectedDatasetCacheEngine {
            inner: Arc::new(RwLock::new(InnerReprojectedDatasetCacheEngine::new())),
        }
    }

    pub fn apply_projection_to_batch(
        &self,
        source_projection_code: impl AsRef<str>,
        target_projection_code: impl AsRef<str>,
        record_batch_name: impl AsRef<str>,
        batch: RecordBatch
    ) -> Result<(), MapError> {

        // Check if projection exists, if it does not, create it. If it is being created by another thread, wait for it to finish
        if !self.inner.read().unwrap().projection_applied_batch_exists(
            target_projection_code.as_ref(),
            record_batch_name.as_ref(),
        ) {
            let mut inner = self.inner.write().unwrap();

            // Check again if projection exists, if it does not, create it. This the second check is needed because another thread could have created it in the meantime
            if !inner.projection_applied_batch_exists(
                target_projection_code.as_ref(),
                record_batch_name.as_ref(),
            ) {
                return inner.apply_projection_to_record_batch(
                    source_projection_code.as_ref(),
                    target_projection_code.as_ref(),
                    record_batch_name.as_ref(),
                    batch
                );
            }
        }

        Ok(())
    }

    pub fn get_projection_applied_batch(
        &self,
        projection: impl AsRef<str>,
        record_batch_name: impl AsRef<str>,
    ) -> Option<RecordBatch> {
        self.inner
            .read()
            .unwrap()
            .get_projection_applied_batch(projection.as_ref(), record_batch_name.as_ref())
    }
}

struct InnerReprojectedDatasetCacheEngine {
    // Map of projections to file name to the record batch of that file
    projections: RwLock<LruCache<String, RecordBatch>>,
}

impl InnerReprojectedDatasetCacheEngine {
    fn new() -> Self {
        let size = NonZeroUsize::new(LRU_CACHE_SIZE).unwrap();
        InnerReprojectedDatasetCacheEngine {
            projections: RwLock::new(LruCache::new(size)),
        }
    }

    fn projection_applied_batch_exists(
        &self,
        projection_code: impl AsRef<str>,
        record_batch_name: impl AsRef<str>,
    ) -> bool {
        let projections = self.projections.read().unwrap();
        let cache_key = get_cache_key(projection_code, record_batch_name);
        projections.contains(&cache_key)
    }

    fn get_projection_applied_batch(
        &self,
        projection_code: impl AsRef<str>,
        record_batch_name: impl AsRef<str>,
    ) -> Option<RecordBatch> {
        // Cloning the record batch is just a shared reference into various arrow buffers, so it is cheap to clone
        let cache_key = get_cache_key(projection_code, record_batch_name);

        // log::info!("Getting projection applied batch for cache key: {}", cache_key);

        let mut projections = self.projections.write().unwrap();

        projections.get(&cache_key).cloned()
    }

    fn apply_projection_to_record_batch(
        &mut self,
        source_projection_code: impl AsRef<str>,
        target_projection_code: impl AsRef<str>,
        record_batch_name: impl AsRef<str>,
        batch: RecordBatch
    ) -> Result<(), MapError> {
        let source_projection_code = source_projection_code.as_ref().to_uppercase();
        let target_projection_code = target_projection_code.as_ref().to_uppercase();

        // log::info!(
        //     "Applying projection from {} to {} for record batch {}",
        //     source_projection_code,
        //     target_projection_code,
        //     record_batch_name.as_ref()
        // );

        let schema = batch.schema();

        schema
            .column_with_name(LATITUDE_COLUMN)
            .ok_or(MapError::Error(String::from(
                format!("Could not find column {LATITUDE_COLUMN} in schema!")
            )))?;

        schema
            .column_with_name(LONGITUDE_COLUMN)
            .ok_or(MapError::Error(String::from(
                format!("Could not find column {LONGITUDE_COLUMN} in schema!")
            )))?;

        // Do the projection applying and then recreate the record batch for that projection
        // This is done to avoid having to do the projection every time the record batch is used
        let latitude_column = misc::cast_to_f64(batch.column_by_name(LATITUDE_COLUMN).unwrap())?;
        let latitude_column = latitude_column.into_iter();

        let longitude_column = misc::cast_to_f64(batch.column_by_name(LONGITUDE_COLUMN).unwrap())?;
        let longitude_column = longitude_column.into_iter();

        let mut lat_vec: Vec<Option<f64>> = Vec::with_capacity(latitude_column.len());
        let mut lng_vec: Vec<Option<f64>> = Vec::with_capacity(longitude_column.len());

        let zipped_iterator = latitude_column.zip(longitude_column);

        for (lat, lng) in zipped_iterator {
            if lat.is_none() || lng.is_none() {
                lng_vec.push(None);
                lat_vec.push(None);
                continue;
            }

            let lat = lat.unwrap();
            let lng = lng.unwrap();

            let mut coordinates = (lng, lat); // X Y

            let res = misc::transform_coordinates(
                &source_projection_code,
                &target_projection_code,
                &mut coordinates,
            );

            if res.is_err() {
                log::error!(
                    "Could not convert coordinates {:?}, target projection: {} \n{}",
                    (lng, lat),
                    target_projection_code,
                    res.err().unwrap()
                );
                lng_vec.push(None);
                lat_vec.push(None);
                continue;
            }

            lng_vec.push(Some(coordinates.0));
            lat_vec.push(Some(coordinates.1));
        }

        let lng_arr = Float64Array::from(lng_vec);
        let lat_arr = Float64Array::from(lat_vec);

        let mut fields = vec![
            Field::new(LONGITUDE_COLUMN, DataType::Float64, true),
            Field::new(LATITUDE_COLUMN, DataType::Float64, true)
        ];

        let mut columns: Vec<Arc<dyn arrow::array::Array>> = vec![
            Arc::new(lng_arr),
            Arc::new(lat_arr)
        ];

        for field in schema.fields() {
            if field.name() != LATITUDE_COLUMN && field.name() != LONGITUDE_COLUMN
            {
                let new_field = Field::new(field.name(), field.data_type().clone(), true);
                fields.push(new_field);

                let col = batch.column_by_name(field.name()).unwrap();
                columns.push(col.clone());
            }
        }

        let schema = Arc::new(arrow::datatypes::Schema::new(fields));

        let new_batch = arrow::record_batch::RecordBatch::try_new(schema,columns)
            .map_err(|e| MapError::Error(format!("Could not create record batch: {}", e)))?;

        let cache_key = get_cache_key(target_projection_code, record_batch_name);

        let mut projections = self.projections.write().unwrap();

        projections.put(cache_key, new_batch);

        Ok(())
    }
}

fn get_cache_key(projection_code: impl AsRef<str>, dataset_name: impl AsRef<str>) -> String {
    format!("{}-{}", projection_code.as_ref(), dataset_name.as_ref()).to_lowercase()
}
