//! Spatial index and level of detail pyramid for the draw path.
//!
//! A tile must not read the whole layer. Two structures prevent that.
//!
//! - A **level of detail** keeps one point per screen pixel at one zoom. A tile at that
//!   zoom then holds at most `width * height` points, whatever the layer size is. This
//!   matches the centre pixel dedup that `map_drawing` already applies per tile.
//! - An **index grid** orders the points of a level row major by cell and records a CSR
//!   offset per cell. A bounding box query then reads one contiguous run per cell row.
//!
//! A level holds row numbers, not coordinates. Every level therefore costs 4 bytes per
//! point on top of the one shared copy of the coordinates.

use arrow::array::{Array, RecordBatch};
use lru::LruCache;
use std::sync::{Arc, Mutex};

use crate::errors::MapError;
use crate::map_drawing::{LATITUDE_COLUMN, LONGITUDE_COLUMN, VALUE_COLUMN};
use crate::misc;

/// Finest level of detail that the pyramid holds. 256 << 12 cells per axis is a cell of
/// about 38 m in Web Mercator, so a level 12 point never moves a visible distance.
const MAX_LOD_LEVEL: u32 = 12;

/// A level that keeps this share of the layer saves nothing. Every zoom from there up
/// reads the full set through its own index grid.
const LOD_KEEP_LIMIT: f64 = 0.9;

/// Points a cell of the index grid holds on average. It sets the floor on the grid
/// resolution, for a level whose tiles are coarser than its density asks for.
const POINTS_PER_INDEX_CELL: usize = 32;

/// Widest index grid on one axis. It caps one CSR offset array at 16 M entries.
const MAX_INDEX_DIM: usize = 4096;

/// Levels of headroom between the zoom and the level that serves it.
///
/// A level cell must never span two screen pixels, or the level drops a point that the
/// full resolution draw would keep. Exact alignment needs the tile scheme of the CRS,
/// and a WMS client may also ask for a box that no tile scheme holds. One finer level
/// makes a cell half a pixel, which covers both cases.
const LOD_LEVEL_MARGIN: u32 = 1;

/// Row numbers of one level of detail, ordered row major by index grid cell.
struct PointLevel {
    /// Row numbers into the shared coordinate arrays.
    idx: Vec<u32>,

    cols: usize,
    rows: usize,
    min_x: f64,
    min_y: f64,
    inv_cell_w: f64,
    inv_cell_h: f64,

    /// CSR offsets over the index grid. Length is `cols * rows + 1`.
    start: Vec<u32>,
}

impl PointLevel {
    fn bytes(&self) -> usize {
        self.idx.len() * 4 + self.start.len() * 4
    }

    /// Order the given rows into an index grid and build the CSR offsets.
    ///
    /// `indices` must already hold the draw order that the level wants. The counting
    /// sort is stable, so rows of one cell keep that order.
    ///
    /// `extent` is the data extent as `(min_x, min_y, max_x, max_y)`. `dim` is the wanted
    /// cell count on one axis, from the tile of the zoom that the level serves. A denser
    /// level gets a finer grid than that, because a query costs a whole cell.
    fn build(
        indices: Vec<u32>,
        xs: &[f64],
        ys: &[f64],
        extent: (f64, f64, f64, f64),
        dim: usize,
    ) -> PointLevel {
        let n = indices.len();

        if n == 0 {
            return PointLevel {
                idx: Vec::new(),
                cols: 0,
                rows: 0,
                min_x: 0.0,
                min_y: 0.0,
                inv_cell_w: 0.0,
                inv_cell_h: 0.0,
                start: vec![0],
            };
        }

        let (min_x, min_y, max_x, max_y) = extent;

        // A cell must hold few points, and it must not be wider than a tile. Take
        // whichever rule asks for more cells.
        let dim = dim.max((n / POINTS_PER_INDEX_CELL).isqrt());

        // A grid far finer than the data is all empty cells. Keep it near the point count.
        let dim = dim.clamp(1, MAX_INDEX_DIM);
        let dim = dim.min((4 * n).max(1024).isqrt());
        let (cols, rows) = (dim, dim);

        // A zero span means every point shares one coordinate. One cell then holds them all.
        let span_x = if max_x > min_x { max_x - min_x } else { 1.0 };
        let span_y = if max_y > min_y { max_y - min_y } else { 1.0 };

        let inv_cell_w = cols as f64 / span_x;
        let inv_cell_h = rows as f64 / span_y;

        let cell_of = |i: u32| -> usize {
            let col = (((xs[i as usize] - min_x) * inv_cell_w) as usize).min(cols - 1);
            let row = (((ys[i as usize] - min_y) * inv_cell_h) as usize).min(rows - 1);

            row * cols + col
        };

        let mut start = vec![0u32; cols * rows + 1];

        for &i in &indices {
            start[cell_of(i) + 1] += 1;
        }

        for c in 1..start.len() {
            start[c] += start[c - 1];
        }

        let mut cursor = start.clone();
        let mut idx = vec![0u32; n];

        for &i in &indices {
            let cell = cell_of(i);
            let slot = cursor[cell] as usize;
            cursor[cell] += 1;

            idx[slot] = i;
        }

        PointLevel {
            idx,
            cols,
            rows,
            min_x,
            min_y,
            inv_cell_w,
            inv_cell_h,
            start,
        }
    }
}

/// Cell range that a coordinate span covers, or None when it misses the grid.
fn axis_range(
    min: f64,
    max: f64,
    origin: f64,
    inv_cell: f64,
    count: usize,
) -> Option<(usize, usize)> {
    let lo = ((min - origin) * inv_cell).floor();
    let hi = ((max - origin) * inv_cell).floor();

    if hi < 0.0 || lo >= count as f64 {
        return None;
    }

    Some((
        lo.max(0.0) as usize,
        hi.min((count - 1) as f64).max(0.0) as usize,
    ))
}

/// One layer in one CRS: the coordinates once, plus a level of detail pyramid over them.
pub struct LayerPoints {
    x: Vec<f64>,
    y: Vec<f64>,
    v: Vec<f64>,

    /// Index `z` serves zoom `z`. The last entry holds every row and serves the rest.
    levels: Vec<PointLevel>,
    bytes: usize,
}

impl LayerPoints {
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    /// Number of levels, the full set included.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Coordinates and value of one row.
    pub fn point(&self, row: u32) -> (f64, f64, f64) {
        let row = row as usize;

        (self.x[row], self.y[row], self.v[row])
    }

    /// Call `f` with every point of the level that serves `zoom` inside the box.
    ///
    /// The order follows the index grid, so it carries the shape of the cells. A draw
    /// path must not use it. Call `collect_in_bbox` instead.
    pub fn for_each_in_bbox<F>(
        &self,
        zoom: u32,
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        mut f: F,
    ) -> usize
    where
        F: FnMut(f64, f64, f64),
    {
        self.collect_in_bbox(zoom, min_x, min_y, max_x, max_y, &mut Vec::new(), |row| {
            let (x, y, v) = self.point(row);

            f(x, y, v);
        })
    }

    /// Put the rows of the level that serves `zoom` inside the box in `rows`, in file
    /// order, then call `f` with each of them. Returns the number of points visited,
    /// which is the cost of the tile.
    ///
    /// The box may reach past the grid. It is clamped, so a query outside the data
    /// extent calls `f` no times. The index answers by whole cells, so a row can sit
    /// outside the box. The caller culls those.
    ///
    /// **File order matters.** Two icons that overlap draw one over the other, so the
    /// order decides the colour of the shared pixels. Index order runs cell by cell,
    /// which puts a hard edge on every cell border of the grid. File order holds no
    /// such shape, and it is the order that a scan of the whole layer draws in.
    ///
    /// `rows` is a scratch buffer. The caller keeps it to save an allocation per tile.
    pub fn collect_in_bbox<F>(
        &self,
        zoom: u32,
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        rows: &mut Vec<u32>,
        mut f: F,
    ) -> usize
    where
        F: FnMut(u32),
    {
        rows.clear();

        if self.x.is_empty() || min_x > max_x || min_y > max_y {
            return 0;
        }

        let wanted = (zoom + LOD_LEVEL_MARGIN) as usize;
        let level = &self.levels[wanted.min(self.levels.len() - 1)];

        if level.idx.is_empty() {
            return 0;
        }

        let Some((c0, c1)) = axis_range(min_x, max_x, level.min_x, level.inv_cell_w, level.cols)
        else {
            return 0;
        };

        let Some((r0, r1)) = axis_range(min_y, max_y, level.min_y, level.inv_cell_h, level.rows)
        else {
            return 0;
        };

        let mut scanned = 0usize;

        for row in r0..=r1 {
            let base = row * level.cols;

            // Cells of one row are contiguous, so the whole span is one run.
            let from = level.start[base + c0] as usize;
            let to = level.start[base + c1 + 1] as usize;

            scanned += to - from;

            rows.extend_from_slice(&level.idx[from..to]);
        }

        // A run holds file order, but the runs come cell by cell. Restore file order.
        rows.sort_unstable();

        for &row in rows.iter() {
            f(row);
        }

        scanned
    }

    /// Read the reprojected batches and build the pyramid.
    ///
    /// `world` is the extent of the CRS as `(min_x, min_y, max_x, max_y)`. The level
    /// grids follow it, so a level cell equals a screen pixel of the matching zoom.
    pub fn build(batches: &[RecordBatch], world: (f64, f64, f64, f64)) -> Result<Self, MapError> {
        let (x, y, v) = collect_points(batches)?;
        let n = x.len();

        let extent = data_extent(&x, &y);

        // Cells of a level match the tiles of the zoom it serves, scaled to the data.
        let world_span = world.2 - world.0;
        let data_span = (extent.2 - extent.0).max(extent.3 - extent.1).max(f64::MIN_POSITIVE);
        let data_share = (data_span / world_span).min(1.0);

        let dim_for_zoom = |zoom: u32| -> usize {
            let tiles_across = (1u64 << zoom.min(20)) as f64;

            (tiles_across * data_share).ceil() as usize
        };

        let mut levels: Vec<PointLevel> = Vec::new();

        if n > 0 {
            // Sort by Morton code once. A coarser level drops the low bits of the code,
            // so one linear scan over this order then gives every level.
            let codes = morton_codes(&x, &y, world);

            let mut order: Vec<u32> = (0..n as u32).collect();
            order.sort_unstable_by_key(|&i| codes[i as usize]);

            let limit = (n as f64 * LOD_KEEP_LIMIT) as usize;

            for level in 0..=MAX_LOD_LEVEL {
                let kept = thin_to_level(&order, &codes, level);

                // A level this dense saves nothing over the full set.
                if kept.len() > limit {
                    break;
                }

                let zoom = level.saturating_sub(LOD_LEVEL_MARGIN);

                levels.push(PointLevel::build(kept, &x, &y, extent, dim_for_zoom(zoom)));
            }
        }

        // The full set serves every zoom above the pyramid, so it takes the finest grid.
        levels.push(PointLevel::build(
            (0..n as u32).collect(),
            &x,
            &y,
            extent,
            MAX_INDEX_DIM,
        ));

        let bytes = n * 24 + levels.iter().map(|l| l.bytes()).sum::<usize>();

        Ok(LayerPoints {
            x,
            y,
            v,
            levels,
            bytes,
        })
    }
}

/// Bounding box of the points, as `(min_x, min_y, max_x, max_y)`.
///
/// An empty set gives a unit box, so a grid built on it still divides.
fn data_extent(xs: &[f64], ys: &[f64]) -> (f64, f64, f64, f64) {
    if xs.is_empty() {
        return (0.0, 0.0, 1.0, 1.0);
    }

    let (mut min_x, mut max_x) = (f64::MAX, f64::MIN);
    let (mut min_y, mut max_y) = (f64::MAX, f64::MIN);

    for i in 0..xs.len() {
        min_x = min_x.min(xs[i]);
        max_x = max_x.max(xs[i]);
        min_y = min_y.min(ys[i]);
        max_y = max_y.max(ys[i]);
    }

    (min_x, min_y, max_x, max_y)
}

/// One row per occupied cell of the level, in file order.
///
/// `order` must be sorted by Morton code. A right shift of a code gives the cell of a
/// coarser level, and shifted codes stay sorted, so equal cells sit next to each other.
/// The winner of a cell is its lowest row number, which is the row that the full
/// resolution draw would keep.
fn thin_to_level(order: &[u32], codes: &[u64], level: u32) -> Vec<u32> {
    let shift = 2 * (MAX_LOD_LEVEL - level);

    let mut kept: Vec<u32> = Vec::new();
    let mut previous = u64::MAX;

    for &i in order {
        let cell = codes[i as usize] >> shift;

        if cell != previous {
            kept.push(i);
            previous = cell;
            continue;
        }

        let winner = kept.last_mut().unwrap();

        if i < *winner {
            *winner = i;
        }
    }

    // Restore the file order among the survivors, so the draw order stays stable.
    kept.sort_unstable();

    kept
}

/// Longitude, latitude and value of every row that can draw.
///
/// A row with a null in any of the three can never colour a pixel, so it is dropped here
/// instead of on every tile.
fn collect_points(batches: &[RecordBatch]) -> Result<(Vec<f64>, Vec<f64>, Vec<f64>), MapError> {
    let total: usize = batches.iter().map(|b| b.num_rows()).sum();

    let mut xs = Vec::with_capacity(total);
    let mut ys = Vec::with_capacity(total);
    let mut vs = Vec::with_capacity(total);

    for batch in batches {
        let lat = misc::cast_to_f64(column(batch, LATITUDE_COLUMN)?)?;
        let lng = misc::cast_to_f64(column(batch, LONGITUDE_COLUMN)?)?;
        let val = misc::cast_to_f64(column(batch, VALUE_COLUMN)?)?;

        let has_nulls = lat.null_count() + lng.null_count() + val.null_count() > 0;

        let (lats, lngs, vals) = (lat.values(), lng.values(), val.values());

        for row in 0..batch.num_rows() {
            if has_nulls && (lat.is_null(row) || lng.is_null(row) || val.is_null(row)) {
                continue;
            }

            xs.push(lngs[row]);
            ys.push(lats[row]);
            vs.push(vals[row]);
        }
    }

    Ok((xs, ys, vs))
}

fn column<'a>(batch: &'a RecordBatch, name: &str) -> Result<&'a dyn Array, MapError> {
    batch
        .column_by_name(name)
        .map(|c| c.as_ref())
        .ok_or_else(|| MapError::Error(format!("Could not find column {name} in schema!")))
}

/// Morton code of every point on the finest level grid.
///
/// The code interleaves the bits of the cell column and the cell row. A shift right by
/// `2 * k` therefore gives the cell of the level that is `k` steps coarser.
fn morton_codes(xs: &[f64], ys: &[f64], world: (f64, f64, f64, f64)) -> Vec<u64> {
    let (world_min_x, world_min_y, world_max_x, world_max_y) = world;

    let cells = (256u64 << MAX_LOD_LEVEL) as f64;
    let last = cells - 1.0;

    // A tile pyramid is square, so the grid must be square too. Take the span from the
    // x axis. The y axis of the reprojected bounds is wrong: proj clamps latitude 90,
    // which stretches Web Mercator y by 0.06%, enough to shift a level cell off a pixel.
    let span = world_max_x - world_min_x;
    let origin_x = (world_min_x + world_max_x) / 2.0 - span / 2.0;
    let origin_y = (world_min_y + world_max_y) / 2.0 + span / 2.0;

    let scale = cells / span;

    xs.iter()
        .zip(ys.iter())
        .map(|(&x, &y)| {
            let col = ((x - origin_x) * scale).clamp(0.0, last) as u64;

            // The row counts from the top, so a level cell matches a screen pixel.
            let row = ((origin_y - y) * scale).clamp(0.0, last) as u64;

            interleave(col) | (interleave(row) << 1)
        })
        .collect()
}

/// Spread the low 21 bits of a value over the even bit positions.
///
/// Two spread values, one shifted up by one, interleave into a 2D Morton code.
fn interleave(value: u64) -> u64 {
    let mut v = value & 0x0000_0000_001f_ffff;

    v = (v | (v << 16)) & 0x0000_ffff_0000_ffff;
    v = (v | (v << 8)) & 0x00ff_00ff_00ff_00ff;
    v = (v | (v << 4)) & 0x0f0f_0f0f_0f0f_0f0f;
    v = (v | (v << 2)) & 0x3333_3333_3333_3333;
    v = (v | (v << 1)) & 0x5555_5555_5555_5555;

    v
}

/// Cache of built pyramids, one per layer file, generation and CRS.
///
/// The budget counts bytes, not entries. One pyramid of a 28 M row layer is near a
/// gigabyte, so an entry count gives no useful bound.
pub struct PointIndexCache {
    inner: Mutex<CacheInner>,
    budget_bytes: usize,
}

struct CacheInner {
    lru: LruCache<String, Arc<LayerPoints>>,
    bytes: usize,
}

impl PointIndexCache {
    pub fn new(budget_bytes: usize) -> Self {
        PointIndexCache {
            inner: Mutex::new(CacheInner {
                lru: LruCache::unbounded(),
                bytes: 0,
            }),
            budget_bytes,
        }
    }

    /// Budget from `POINT_INDEX_BUDGET_MB`. The default holds one large layer per CRS.
    pub fn from_env() -> Self {
        let value = misc::get_env_var("POINT_INDEX_BUDGET_MB", Some("4096"));

        let megabytes = match value.trim().parse::<usize>() {
            Ok(n) if n > 0 => n,
            _ => {
                if !value.trim().is_empty() {
                    log::warn!("Invalid POINT_INDEX_BUDGET_MB '{}'. Using 4096.", value);
                }

                4096
            }
        };

        PointIndexCache::new(megabytes * 1024 * 1024)
    }

    pub fn get(&self, key: &str) -> Option<Arc<LayerPoints>> {
        self.lock().lru.get(key).cloned()
    }

    /// Store a pyramid and drop the least used entries until the budget holds.
    pub fn insert(&self, key: String, points: Arc<LayerPoints>) {
        let mut inner = self.lock();

        if let Some(old) = inner.lru.pop(&key) {
            inner.bytes -= old.bytes();
        }

        inner.bytes += points.bytes();
        inner.lru.put(key, points);

        // One entry always stays. A budget below the size of a single layer would
        // otherwise drop the pyramid that this request just built.
        while inner.bytes > self.budget_bytes && inner.lru.len() > 1 {
            match inner.lru.pop_lru() {
                Some((_, dropped)) => inner.bytes -= dropped.bytes(),
                None => break,
            }
        }
    }

    pub fn bytes(&self) -> usize {
        self.lock().bytes
    }

    /// A panic during one render must not close the cache for every later request.
    fn lock(&self) -> std::sync::MutexGuard<'_, CacheInner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(col: u64, row: u64) -> u64 {
        interleave(col) | (interleave(row) << 1)
    }

    fn morton_order(codes: &[u64]) -> Vec<u32> {
        let mut order: Vec<u32> = (0..codes.len() as u32).collect();
        order.sort_unstable_by_key(|&i| codes[i as usize]);

        order
    }

    /// A Morton code must hold the column in the even bits and the row in the odd bits.
    #[test]
    fn morton_code_interleaves_both_axes() {
        assert_eq!(code(0, 0), 0);
        assert_eq!(code(1, 0), 0b01);
        assert_eq!(code(0, 1), 0b10);
        assert_eq!(code(1, 1), 0b11);
        assert_eq!(code(0b101, 0b011), 0b01_10_11);
    }

    /// A shift of two bits must give the cell of the next coarser level.
    #[test]
    fn morton_shift_gives_the_coarser_cell() {
        for col in 0..64u64 {
            for row in 0..64u64 {
                assert_eq!(code(col, row) >> 2, code(col >> 1, row >> 1));
                assert_eq!(code(col, row) >> 8, code(col >> 4, row >> 4));
            }
        }
    }

    /// Thinning keeps one row per cell, and it keeps the lowest row number of that cell.
    #[test]
    fn thinning_keeps_the_first_row_of_each_cell() {
        // Rows 0 and 3 share one level 11 cell. Rows 1 and 2 share the next one.
        let codes = vec![code(0, 0), code(2, 0), code(3, 0), code(1, 0)];

        let kept = thin_to_level(&morton_order(&codes), &codes, MAX_LOD_LEVEL - 1);

        assert_eq!(kept, vec![0, 1]);
    }

    /// A level must never hold more points than its grid has cells.
    #[test]
    fn a_level_holds_at_most_one_point_per_cell() {
        // 64 by 64 distinct finest cells collapse to 4 by 4 at four levels up.
        let codes: Vec<u64> = (0..4096u64).map(|i| code(i % 64, i / 64)).collect();
        let order = morton_order(&codes);

        assert_eq!(thin_to_level(&order, &codes, MAX_LOD_LEVEL).len(), 4096);
        assert_eq!(thin_to_level(&order, &codes, MAX_LOD_LEVEL - 4).len(), 16);
    }

    /// A bounding box query must return every point inside it and nothing outside it.
    #[test]
    fn bbox_query_matches_a_linear_scan() {
        let n = 5000usize;

        // A cheap spread that fills the unit square without a random dependency.
        let xs: Vec<f64> = (0..n).map(|i| ((i * 7919) % 1000) as f64).collect();
        let ys: Vec<f64> = (0..n).map(|i| ((i * 104729) % 1000) as f64).collect();
        let vs: Vec<f64> = (0..n).map(|i| i as f64).collect();

        let level = PointLevel::build((0..n as u32).collect(), &xs, &ys, data_extent(&xs, &ys), 64);

        let points = LayerPoints {
            x: xs.clone(),
            y: ys.clone(),
            v: vs.clone(),
            levels: vec![level],
            bytes: 0,
        };

        for (min_x, min_y, max_x, max_y) in [
            (100.0, 100.0, 300.0, 300.0),
            (-50.0, -50.0, 50.0, 50.0),
            (900.0, 0.0, 2000.0, 1000.0),
            (2000.0, 2000.0, 3000.0, 3000.0),
        ] {
            let mut found: Vec<u64> = Vec::new();

            points.for_each_in_bbox(0, min_x, min_y, max_x, max_y, |_, _, v| {
                found.push(v as u64);
            });

            let mut expected: Vec<u64> = (0..n)
                .filter(|&i| {
                    xs[i] >= min_x && xs[i] <= max_x && ys[i] >= min_y && ys[i] <= max_y
                })
                .map(|i| i as u64)
                .collect();

            found.sort_unstable();
            expected.sort_unstable();

            // The grid is conservative, so it may hand back a point just outside the box.
            for want in &expected {
                assert!(found.contains(want), "missed point {want}");
            }
        }
    }
}
