use arrow::{array::{Float64Array, RecordBatch}, datatypes::{DataType, Field}};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{
    errors::MapError,
    map_drawing::{LATITUDE_COLUMN, LONGITUDE_COLUMN, VALUE_COLUMN},
    misc::{self, CoordinateTransform},
};

/// Reproject the coordinates of a batch and keep only longitude, latitude and value.
///
/// All other columns are dropped, so the result stays small.
pub fn reproject_batch(
    source_projection_code: &str,
    target_projection_code: &str,
    batch: RecordBatch,
) -> Result<RecordBatch, MapError> {
    let schema = batch.schema();

    schema
        .column_with_name(LATITUDE_COLUMN)
        .ok_or(MapError::Error(format!(
            "Could not find column {LATITUDE_COLUMN} in schema!"
        )))?;

    schema
        .column_with_name(LONGITUDE_COLUMN)
        .ok_or(MapError::Error(format!(
            "Could not find column {LONGITUDE_COLUMN} in schema!"
        )))?;

    // Resolve the projection pair once. A per point lookup takes a lock and clones a Proj.
    let transform = CoordinateTransform::new(source_projection_code, target_projection_code)
        .map_err(MapError::Error)?;

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

        if let Err(e) = transform.apply(&mut coordinates) {
            log::error!(
                "Could not convert coordinates {:?}, target projection: {} \n{}",
                (lng, lat),
                target_projection_code,
                e
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

    if let Some(col) = batch.column_by_name(VALUE_COLUMN) {
        let field = schema.field_with_name(VALUE_COLUMN).unwrap();
        fields.push(Field::new(VALUE_COLUMN, field.data_type().clone(), true));
        columns.push(col.clone());
    }

    let schema = Arc::new(arrow::datatypes::Schema::new(fields));

    arrow::record_batch::RecordBatch::try_new(schema, columns)
        .map_err(|e| MapError::Error(format!("Could not create record batch: {}", e)))
}

/// One gate per set of cached batches. It keeps duplicate decodes to one.
///
/// A browser opens a map with 6 to 12 tiles of the same layer and the same CRS. Those
/// requests share every cached batch, so only the first one must read the parquet file.
/// The others wait on the gate and then find the batches in the cache.
///
/// The mutex is a std mutex, because the draw path runs on the blocking pool. A tokio
/// mutex there blocks a runtime thread.
pub struct DecodeGates {
    gates: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl DecodeGates {
    pub fn new() -> Self {
        DecodeGates {
            gates: Mutex::new(HashMap::new()),
        }
    }

    /// Gate of one cache key. Lock it around the decode, then pass it to `release`.
    pub fn acquire(&self, key: &str) -> Arc<Mutex<()>> {
        self.lock_gates()
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Drop the gate of a key once no other thread holds it. Keeps the map bounded,
    /// because every refresh of a layer makes a new key.
    pub fn release(&self, key: &str, gate: Arc<Mutex<()>>) {
        let mut gates = self.lock_gates();

        // Two references means this caller and the map. A clone can only happen under
        // the same lock, so the count cannot change here.
        if Arc::strong_count(&gate) == 2 {
            gates.remove(key);
        }
    }

    /// A panic in one render must not close the gate for every later request.
    fn lock_gates(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<Mutex<()>>>> {
        self.gates.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Lock a gate and survive a poisoned mutex.
pub fn lock_gate(gate: &Mutex<()>) -> std::sync::MutexGuard<'_, ()> {
    gate.lock().unwrap_or_else(|e| e.into_inner())
}
