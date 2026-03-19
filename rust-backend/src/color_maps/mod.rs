use core::fmt;
use image::{Rgba};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use statrs::generate;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{image_utils, misc};

const COLOR_ALPHA: u8 = 255;

lazy_static! {
    static ref COLOR_MAPS_CACHE: Mutex<HashMap<String, ColorMap>> = Mutex::new(HashMap::new());
}

#[derive(Serialize, Deserialize)]
pub struct ColorMapsConfig {
    colormaps: Vec<ColorMapData>,
}

impl fmt::Debug for ColorMapsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorMapsConfig")
            .field("colormaps", &self.colormaps)
            .finish()
    }
}

impl ColorMapsConfig {
    pub fn get_named(&self, name: &str) -> Option<&ColorMapData> {
        self.colormaps.iter().find(|&x| x.name == name)
    }

    pub fn get(&self, index: usize) -> Option<&ColorMapData> {
        self.colormaps.get(index)
    }

    pub fn all(&self) -> &Vec<ColorMapData> {
        &self.colormaps
    }

    pub fn get_names(&self) -> Vec<String> {
        self.colormaps.iter().map(|x| x.name.clone()).collect()
    }

    pub fn load() -> Option<ColorMapsConfig> {
        let config_dir = misc::get_env_var("CONFIG_DIR", Some("../config"));

        let color_maps_config_path = format!("{}/colormaps.json", config_dir);

        if color_maps_config_path.is_empty() {
            log::error!("COLOR_MAPS_CONFIG_LOCATION environment variable is not set");
            return None;
        }

        let file_contents = &std::fs::read_to_string(color_maps_config_path).unwrap();

        let color_maps_config = serde_json::from_str(file_contents).unwrap();


        Some(color_maps_config)
    }
}

// -------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")] // makes JSON use "lab", "linear", "nearest"
pub enum Interpolation {
    Lab,
    Linear,
    Nearest,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ColorMapData {
    pub name: String,
    pub description: Option<String>,
    pub interpolation: Interpolation,
    pub scale: Vec<(f64, [u8; 3])>,
}

impl fmt::Debug for ColorMapData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorMapData")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("interpolation", &self.interpolation)
            .field("length", &self.scale.len())
            .finish()
    }
}

#[derive(Clone)]
pub struct ColorMap {
    color_map_data: Arc<ColorMapData>,
    interpolation: Interpolation,
    colors: Vec<(f64, Rgba<u8>)>,
    lab_colors: Vec<(f64, (f64, f64, f64))>,
    log_space: Vec<f64>,
    log: bool,
    min_value: f64,
    max_value: f64,
}

impl ColorMap {
    /// Get a named colormap from the cache or create a new one
    pub fn get_named(
        name: &str,
        min_value: f64,
        max_value: f64,
        log: Option<bool>,
    ) -> Option<ColorMap> {
        let log = log.unwrap_or(false);

        let cache_key = format!("{}-{}-{}-{}", name, min_value, max_value, log);

        {
            let cache = COLOR_MAPS_CACHE.lock().unwrap();
            if let Some(color_map) = cache.get(&cache_key) {
                return Some(color_map.clone());
            }
        }

        let color_maps_config = match ColorMapsConfig::load() {
            Some(c) => c,
            None => {
                log::error!("Failed to load color maps config");
                return None;
            }
        };
        

        let color_map_data = color_maps_config.get_named(name);

        if color_map_data.is_none() {
            return None;
        }

        let color_map_data: Arc<ColorMapData> = Arc::new(color_map_data.unwrap().clone());

        let color_map = ColorMap::new(color_map_data, min_value, max_value, Some(log));

        {
            let mut cache = COLOR_MAPS_CACHE.lock().unwrap();
            cache.insert(cache_key, color_map.clone());
        }

        Some(color_map)
    }

    pub fn ref_self<'b>(&'b self) -> &'b Self {
        &self
    }

    pub fn new(
        color_map_data: Arc<ColorMapData>,
        min_value: f64,
        max_value: f64,
        log: Option<bool>,
    ) -> ColorMap {
        let steps = color_map_data.scale.len();
        let log = log.unwrap_or(false);

        let mut colors: Vec<(f64, Rgba<u8>)> = Vec::with_capacity(steps);
        let mut lab_colors: Vec<(f64, (f64, f64, f64))> = Vec::with_capacity(steps);

        for i in 0..steps {
            let step = color_map_data.scale[i];
            let color = step.1;
            let color_rgba = Rgba([color[0], color[1], color[2], COLOR_ALPHA]);
            let color_lab = image_utils::rgb_to_lab(&color_rgba);

            colors.push((step.0, color_rgba.to_owned()));
            lab_colors.push((step.0, color_lab));
        }

        let log_space = generate::log_spaced(
            steps,
            min_value.log10(), //if below 0 will return NaN
            max_value.log10(),
        );

        let interpolation: Interpolation = color_map_data.interpolation;

        Self {
            color_map_data,
            interpolation,
            colors,
            lab_colors,
            log_space,
            log,
            min_value,
            max_value,
        }
    }

    pub fn query(&self, value: f64) -> Rgba<u8> {
        if self.colors.is_empty() {
            return Rgba([0, 0, 0, 255]); // fallback
        }

        if self.colors.len() == 1 {
            return self.colors[0].1; // only one color in the scale
        }
        
        // Normalize value to [0,1]
        let normalized_value =
            ((value - self.min_value) / (self.max_value - self.min_value)).clamp(0.0, 1.0);

        if self.log {
            self.get_color_logspace(normalized_value)
        } else {
            self.get_color(normalized_value)
        }
    }

    fn get_color(&self, normalized_value: f64) -> Rgba<u8> {
        // Binary search: find the first step whose position is > normalized_value
        let idx = self.colors.partition_point(|(step, _)| *step <= normalized_value);

        if idx == 0 {
            return self.colors[0].1;
        }
        if idx >= self.colors.len() {
            return self.colors.last().unwrap().1;
        }

        let lower = idx - 1;
        let upper = idx;
        let step_lower = self.colors[lower].0;
        let step_upper = self.colors[upper].0;

        let interpolation_fraction = if (step_upper - step_lower).abs() < f64::EPSILON {
            0.0
        } else {
            (normalized_value - step_lower) / (step_upper - step_lower)
        };

        self.interpolate(lower, upper, interpolation_fraction)
    }

    fn get_color_logspace(&self, normalized_value: f64) -> Rgba<u8> {
        // Binary search: find the first log_space entry > normalized_value
        let idx = self.log_space.partition_point(|&step| step <= normalized_value);

        if idx == 0 {
            return self.colors[0].1;
        }
        if idx >= self.log_space.len() {
            return self.colors.last().unwrap().1;
        }

        let lower = idx - 1;
        let upper = idx;
        let step_lower = self.log_space[lower];
        let step_upper = self.log_space[upper];

        let interpolation_fraction = if (step_upper - step_lower).abs() < f64::EPSILON {
            0.0
        } else {
            (normalized_value - step_lower) / (step_upper - step_lower)
        };

        self.interpolate(lower, upper, interpolation_fraction)
    }

    /// Build a pre-computed lookup table of `size` entries spanning [min_value, max_value].
    /// Returns packed u32 RGBA values for direct use in the rendering loop.
    pub fn build_lut(&self, size: usize) -> Vec<u32> {
        let mut lut = Vec::with_capacity(size);
        for i in 0..size {
            let value = self.min_value + (self.max_value - self.min_value) * (i as f64 / (size - 1) as f64);
            let rgba = self.query(value);
            let [r, g, b, a] = rgba.0;
            lut.push(((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | (a as u32));
        }
        lut
    }

    pub fn get_min_value(&self) -> f64 {
        self.min_value
    }

    pub fn get_max_value(&self) -> f64 {
        self.max_value
    }

    fn interpolate(
        &self,
        lower_index: usize,
        upper_index: usize,
        interpolation_fraction: f64,
    ) -> Rgba<u8> {
        match self.interpolation {
            Interpolation::Nearest => {
                //return Nearest color in the scale
                if interpolation_fraction < 0.5 {
                    return self.colors[lower_index].1;
                } else {
                    return self.colors[upper_index].1;
                }
            }
            Interpolation::Lab => image_utils::lab_color_interpolation(
                self.lab_colors[lower_index].1,
                self.lab_colors[upper_index].1,
                interpolation_fraction,
            ),
            Interpolation::Linear => image_utils::linear_color_interpolation(
                &self.colors[lower_index].1,
                &self.colors[upper_index].1,
                interpolation_fraction,
            ),
        }
    }
}

impl<'a> fmt::Debug for ColorMap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorMap")
            .field("interpolation", &self.interpolation)
            .field("log", &self.log)
            .field("min_value", &self.min_value)
            .field("max_value", &self.max_value)
            .field("color_map_data", &self.color_map_data)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_colormaps() {
        let color_map = ColorMap::get_named("rainbow", 0.0, 40.0, Some(false));

        assert!(color_map.is_some());

        let color_map = color_map.unwrap();

        println!("{:?}", color_map);
    }
}
