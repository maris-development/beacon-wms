use crate::{boundingbox::BoundingBox};
use crate::errors::MapError;
use arrow::{array::{Array, PrimitiveArray, AsArray, Float32Array, Float64Array, Int64Array, Int32Array, StringArray}};
use arrow::datatypes::{Float32Type, Float64Type, Int16Type, Int32Type, Int8Type};
use arrow::datatypes::{Int64Type, TimeUnit};
use chrono::{Datelike, Local};
use font_loader::system_fonts;
use image::{Rgba, RgbaImage};
use proj4rs::Proj;
use rand::Rng;
use rusttype::Font;
use std::collections::HashMap;
use std::path::{PathBuf};
use std::string::ToString;
use std::sync::{Arc, RwLock};
use std::{
    env,
    fs::{self},
}; // Brings the ToString trait into scope

use lazy_static::lazy_static;
use log4rs::{
    append::{console::ConsoleAppender, file::FileAppender},
    config::{Appender, Root},
    encode::pattern::PatternEncoder,
    Config,
};
use arrow::array::{
    TimestampSecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
};
use chrono::{DateTime};
use serde_json::Value;
use sha2::{Sha256, Digest};

pub const ARCTIC_MIN_LATITUDE: f64 = 60.0;
pub const ANTARCTIC_MAX_LATITUDE: f64 = -60.0;

pub const MERCATOR_MAX_LATITUDE: f64 = 85.06;
pub const MERCATOR_MIN_LATITUDE: f64 = -85.06;

// Initialize the cache
lazy_static! {
    static ref PROJ_CACHE: RwLock<HashMap<String, Proj>> = RwLock::new(HashMap::new());
}

pub fn get_string_value(col: &Arc<dyn Array>, row_idx: usize) -> String {
    if let Some(float_arr) = col.as_any().downcast_ref::<Float64Array>() {
        float_arr.value(row_idx).to_string()

    } else if let Some(float_arr) = col.as_any().downcast_ref::<Float32Array>() {
        float_arr.value(row_idx).to_string()

    } else if let Some(int_arr) = col.as_any().downcast_ref::<Int64Array>() {
        int_arr.value(row_idx).to_string()

    } else if let Some(int_arr) = col.as_any().downcast_ref::<Int32Array>() {
        int_arr.value(row_idx).to_string()

    } else if let Some(str_arr) = col.as_any().downcast_ref::<StringArray>() {
        str_arr.value(row_idx).to_owned()

    } else if let Some(bool_arr) = col.as_any().downcast_ref::<arrow::array::BooleanArray>() {
        bool_arr.value(row_idx).to_string()

    } else if let Some(timestamp_arr) = col.as_any().downcast_ref::<TimestampSecondArray>() {
        let ts = timestamp_arr.value(row_idx);
        DateTime::from_timestamp(ts, 0)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| "invalid".to_string())

    } else if let Some(timestamp_arr) = col.as_any().downcast_ref::<TimestampMillisecondArray>() {
        let ts = timestamp_arr.value(row_idx);
        let secs = ts / 1000;
        let nsecs = ((ts % 1000) * 1_000_000) as u32;
        DateTime::from_timestamp(secs, nsecs)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| "invalid".to_string())

    } else if let Some(timestamp_arr) = col.as_any().downcast_ref::<TimestampNanosecondArray>() {
        let ts = timestamp_arr.value(row_idx);
        let secs = ts / 1_000_000_000;
        let nsecs = (ts % 1_000_000_000) as u32;
        DateTime::from_timestamp(secs, nsecs)
            .map(|dt| dt.to_rfc3339())
            .unwrap_or_else(|| "invalid".to_string())

    } else {
        log::warn!("Unsupported column data type: {:?}", col.data_type());
        "unsupported".to_string()
    }
}


pub fn transform_coordinates(
    source_projection_code: &str,
    target_projection_code: &str,
    coordinates: &mut (f64, f64),
) -> Result<(), String> {
    //make both uppercase:
    let source_projection_code = source_projection_code.to_uppercase();
    let source_projection_code = source_projection_code.as_str();
    let target_projection_code = target_projection_code.to_uppercase();
    let target_projection_code = target_projection_code.as_str();

    if source_projection_code == target_projection_code {
        return Ok(());
    }

    let target_projection_code = match target_projection_code {
        "EPSG:900913" | "epsg:900913" => "EPSG:3857",
        _ => target_projection_code,
    };

    let source_projection = get_projection(source_projection_code)?;
    let target_projection = get_projection(target_projection_code)?;

    match source_projection_code {
        "EPSG:4326" | "epsg:4326" => match target_projection_code {
            "EPSG:3857" | "epsg:3857" => {
                if coordinates.1 < MERCATOR_MIN_LATITUDE {
                    coordinates.1 = MERCATOR_MIN_LATITUDE;
                }
                if coordinates.1 > MERCATOR_MAX_LATITUDE {
                    coordinates.1 = MERCATOR_MAX_LATITUDE;
                }
            }
            "EPSG:3995" | "epsg:3395" => {
                if coordinates.1 < ARCTIC_MIN_LATITUDE {
                    coordinates.1 = ARCTIC_MIN_LATITUDE;
                }
            }
            "EPSG:3031" | "epsg:3031" => {
                if coordinates.1 > ANTARCTIC_MAX_LATITUDE {
                    coordinates.1 = ANTARCTIC_MAX_LATITUDE;
                }
            }
            _ => {}
        },
        _ => {}
    }

    match source_projection_code {
        // some projections that use degrees, is there a better way to do this?
        "epsg:4326" | "epsg:4269" | "epsg:4322" | "epsg:4283" | "epsg:4214" | "epsg:4231"
        | "epsg:3995" | "epsg:3031" | "EPSG:4326" | "EPSG:4269" | "EPSG:4322" | "EPSG:4283"
        | "EPSG:4214" | "EPSG:4231" | "EPSG:3995" | "EPSG:3031" => {
            deg_to_rad(coordinates);
        }
        _ => {}
    }

    proj4rs::transform::transform(&source_projection, &target_projection, coordinates).map_err(
        |e| {
            String::from(format!(
                "Could not convert coordinates: {:?}, {:?}",
                coordinates, e
            ))
        },
    )?;

    // proj4rs outputs radians for degree-based target CRS, convert back to degrees
    match target_projection_code {
        "EPSG:4326" | "EPSG:4269" | "EPSG:4322" | "EPSG:4283"
        | "EPSG:4214" | "EPSG:4231" | "EPSG:3995" | "EPSG:3031" => {
            rad_to_deg(coordinates);
        }
        _ => {}
    }

    Ok(())
}

pub fn rad_to_deg(coordinates: &mut (f64, f64)) {
    coordinates.0 = coordinates.0.to_degrees();
    coordinates.1 = coordinates.1.to_degrees();
}

pub fn deg_to_rad(coordinates: &mut (f64, f64)) {
    coordinates.0 = coordinates.0.to_radians();
    coordinates.1 = coordinates.1.to_radians();
}

pub fn degrees_per_pixel_to_zoom(degrees_per_pixel: f64, tile_size: Option<u32>) -> u32 {
    let tile_size = tile_size.unwrap_or(256) as f64;
    let max_zoom = 20;

    // Compute the ideal zoom level as a float
    let zoom = (360.0 / (tile_size * degrees_per_pixel)).log2();

    // Round to the nearest integer and clamp
    let closest = zoom.round()
        .clamp(1.0, max_zoom as f64)
        as u32;

    closest
}

// calculate the scale factor based on the degree per pixel -> scalefactor is used to scale the pointsize instead of using zoomlevel
pub fn calculate_scale_factor(degree_per_pixel: f64) -> f64 {
    let factor = 1.0 / degree_per_pixel * 10.0;
    factor / 14.0
}

pub fn calculate_point_radius(zoom: u32, min_size: f64, max_size: f64) -> i32 {
    let max_zoom = 20.0;
    let growth_factor = (max_zoom as f64) / ((max_size / min_size).log2());
    let diameter = min_size * (2.0f64).powf(zoom as f64 / growth_factor);
    (diameter / 2.0).round() as i32
}

pub fn get_projection(epsg_code: &str) -> Result<Proj, String> {
    //alias for EPSG: codes
    let epsg_code = match epsg_code {
        "EPSG:900913" => "EPSG:3857",
        _ => epsg_code,
    };

    // Check if the projection is already in the cache
    {
        let cache = PROJ_CACHE.read().unwrap();
        if let Some(proj) = cache.get(epsg_code) {
            return Ok(proj.clone());
        }
    }

    let result = match epsg_code {
        "EPSG:4326" | "epsg:4326" => Proj::from_epsg_code(4326),
        "EPSG:3857" | "epsg:3857" => Proj::from_epsg_code(3857),
        "EPSG:32633" | "epsg:32633" => Proj::from_epsg_code(32633),
        "EPSG:27700" | "epsg:27700" => Proj::from_epsg_code(27700),
        "EPSG:3413" | "epsg:3413" => Proj::from_epsg_code(3413),
        "EPSG:32718" | "epsg:32718" => Proj::from_epsg_code(32718),
        "EPSG:4231" | "epsg:4231" => Proj::from_epsg_code(4231),
        "EPSG:3995" | "epsg:3995" => Proj::from_epsg_code(3995),
        "EPSG:3031" | "epsg:3031" => Proj::from_epsg_code(3031),

        _ => {
            let proj_definition = get_projection_definition(epsg_code)
                .ok_or(format!("Could not find projection: {}", epsg_code))?;
            let proj_result = Proj::from_proj_string(proj_definition);

            if proj_result.is_err() {
                return Err(format!("Could not create projection: {}", epsg_code));
            }

            proj_result
        }
    };

    // If successful, add the new projection to the cache
    let result = result.map_err(|e| e.to_string())?;
    {
        let mut cache = PROJ_CACHE.write().unwrap();
        cache.insert(epsg_code.to_string(), result.clone());
    }

    Ok(result)
}

pub fn get_projection_definition(epsg_code: &str) -> Option<&str> {
    let projections: HashMap<&str, &str> = [
        ("EPSG:4326", "+proj=longlat +ellps=WGS84 +datum=WGS84 +no_defs"),
        ("EPSG:3857", "+proj=merc +lon_0=0 +k=1 +x_0=0 +y_0=0 +datum=WGS84 +units=m +no_defs +over"),
        ("EPSG:32633", "+proj=utm +zone=33 +datum=WGS84 +units=m +no_defs"),
        ("EPSG:27700", "+proj=tmerc +lat_0=49 +lon_0=-2 +k=0.9996012717 +x_0=400000 +y_0=-100000 +ellps=airy +datum=OSGB36 +units=m +no_defs"),
        ("EPSG:3413", "+proj=stere +lat_0=90 +lat_ts=70 +lon_0=-45 +k=1 +x_0=0 +y_0=0 +datum=WGS84 +units=m +no_defs"),
        ("EPSG:32718", "+proj=utm +zone=18 +south +datum=WGS84 +units=m +no_defs"),
    ].iter().cloned().collect();

    projections.get(epsg_code).copied()
}

pub fn cast_to_f64(arr: &dyn Array) -> Result<PrimitiveArray<Float64Type>, MapError> {
    match arr.data_type() {
        arrow::datatypes::DataType::Int8 => Ok(arr
            .as_primitive::<Int8Type>()
            .unary::<_, Float64Type>(|x| x as f64)),
        arrow::datatypes::DataType::Int16 => Ok(arr
            .as_primitive::<Int16Type>()
            .unary::<_, Float64Type>(|x| x as f64)),
        arrow::datatypes::DataType::Int32 => Ok(arr
            .as_primitive::<Int32Type>()
            .unary::<_, Float64Type>(|x| x as f64)),
        arrow::datatypes::DataType::Int64 => Ok(arr
            .as_primitive::<Int64Type>()
            .unary::<_, Float64Type>(|x| x as f64)),
        arrow::datatypes::DataType::Float32 => Ok(arr
            .as_primitive::<Float32Type>()
            .unary::<_, Float64Type>(|x| x as f64)),
        arrow::datatypes::DataType::Float64 => Ok(arr.as_primitive::<Float64Type>().clone()),

        arrow::datatypes::DataType::Timestamp(TimeUnit::Second, None) => Ok(arr
            .as_primitive::<arrow::datatypes::TimestampSecondType>()
            .unary::<_, Float64Type>(|x| x as f64)),

        arrow::datatypes::DataType::Timestamp(TimeUnit::Millisecond, None) => Ok(arr
            .as_primitive::<arrow::datatypes::TimestampMillisecondType>()
            .unary::<_, Float64Type>(|x| x as f64)),

        arrow::datatypes::DataType::Timestamp(TimeUnit::Nanosecond, None) => Ok(arr
            .as_primitive::<arrow::datatypes::TimestampNanosecondType>()
            .unary::<_, Float64Type>(|x| x as f64)),

        unsupported_type => Err(MapError::Error(format!(
            "Unsupported column data type: {:?}",
            unsupported_type
        ))),
    }
}

pub fn pixel_offset_to_coordinates(
    bbox: &BoundingBox,
    image_dimensions: (u32, u32),
    pixel_offset: (u32, u32),
) -> (f64, f64) {
    let (pixel_x, pixel_y) = pixel_offset;
    let (image_width, image_height) = image_dimensions;

    let x = bbox.get_min_x() + (pixel_x as f64 / image_width as f64) * bbox.get_width();
    let y = bbox.get_min_y()
        + ((image_height as f64 - pixel_y as f64) / image_height as f64) * bbox.get_height();

    (x, y)
}

/// Convert a mercator coordinate to a pixel coordinate within the image representing the bounding box.
pub fn coordinates_to_pixel_offset(
    bbox: &BoundingBox,
    image_size: (u32, u32),
    coordinates: (f64, f64),
) -> (i32, i32) {
    let (mut x, y) = coordinates;
    let (image_width, image_height) = image_size;

    let world_width = bbox.get_max_bounds().get_width();

    // if the x coordinate is outside the world box, wrap it around to the other side of the world
    if f_max(x, bbox.get_center_x()) - f_min(x, bbox.get_center_x()) > world_width / 2.0 {
        if x < 0.0 {
            x += world_width;
        } else {
            x -= world_width;
        }
    }

    let pixel_x =
        (x - bbox.get_min_x()) / (bbox.get_max_x() - bbox.get_min_x()) * image_width as f64;

    let pixel_y = image_height as f64
        - ((y - bbox.get_min_y()) / (bbox.get_max_y() - bbox.get_min_y()) * image_height as f64);

    (pixel_x as i32, pixel_y as i32)
}

pub fn f_min(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}

pub fn f_max(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

pub fn get_tile_bounds(x: i32, y: i32, zoom: i32) -> BoundingBox {
    let bbox = BoundingBox::new(
        tile_to_lon(x, zoom),
        tile_to_lat(y + 1, zoom),
        tile_to_lon(x + 1, zoom),
        tile_to_lat(y, zoom),
        "EPSG:4326",
    );

    bbox
}

pub fn tile_to_lat(y: i32, zoom: i32) -> f64 {
    let pi = std::f64::consts::PI;
    let n = pi - 2.0 * pi * (y as f64) / 2.0_f64.powi(zoom);
    return 180.0 / pi * (0.5 * (n.exp() - (-1.0 * n).exp())).atan();
}

pub fn tile_to_lon(x: i32, zoom: i32) -> f64 {
    return (x as f64) / 2.0_f64.powi(zoom) * 360.0 - 180.0;
}

/// Check if a point is inside the image.
pub fn inside_image(image: &RgbaImage, point: (i32, i32)) -> bool {
    let (point_x, point_y) = point;
    let (image_width, image_height) = image.dimensions();
    point_x >= 0 && point_x < image_width as i32 && point_y >= 0 && point_y < image_height as i32
}

#[allow(dead_code)]
/// Draw a circle on an image, use mageproc::drawing::draw_filled_circle for nicer results.
pub fn draw_circle(
    image: &mut RgbaImage,
    center: (i32, i32),
    radius: i32,
    opt_color: Option<Rgba<u8>>,
) {
    let red = Rgba([255u8, 0u8, 0u8, 255u8]);
    let color = opt_color.unwrap_or(red);
    let diameter = radius * 2;

    let (center_x, center_y) = center;

    let x = center_x as i32 - radius;
    let y = center_y as i32 - radius;

    // pub fn draw_filled_circle<I>(
    //     image: &I,
    //     center: (i32, i32),
    //     radius: i32,
    //     color: I::Pixel
    // ) -> Image<I::Pixel>
    // *image = imageproc::drawing::draw_filled_circle(image, center, radius, color);

    for i in 0..diameter {
        for j in 0..diameter {
            let point = (x + i, y + j);

            if inside_image(image, point) && inside_circle(center, point, radius) {
                image.put_pixel(point.0 as u32, point.1 as u32, color);
            }
        }
    }
}

/// Check if a point is inside a circle of radius `radius` centered at `center`.
pub fn inside_circle(center: (i32, i32), point: (i32, i32), radius: i32) -> bool {
    let (center_x, center_y) = center;
    let (point_x, point_y) = point;

    let dx = center_x - point_x;
    let dy = center_y - point_y;

    let distance_squared = dx * dx + dy * dy;

    distance_squared <= (radius * radius)
}

#[allow(dead_code)]
/// Debug function to print the x, y, and zoom level on the tile image
pub fn print_xyz_on_image(x: i32, y: i32, zoom: i32, image: &mut RgbaImage) {
    let font_property = system_fonts::FontPropertyBuilder::new()
        .family("FreeMono")
        .build();
    let (font_data, _) = system_fonts::get(&font_property).unwrap();
    let msg = format!("{} {} {}", x, y, zoom);
    let font = Font::try_from_bytes(&font_data).unwrap();
    let color = Rgba([0u8, 0u8, 0u8, 255u8]);

    let modified_image = imageproc::drawing::draw_text(
        image,
        color,
        0,
        0,
        rusttype::Scale::uniform(20.0),
        &font,
        msg.as_str(),
    );

    *image = modified_image;
}

#[allow(dead_code)]
/// Debug function to print current bounding box on the image.
pub fn print_bbox_on_image(bbox: &BoundingBox, image: &mut RgbaImage) {
    let font_property = system_fonts::FontPropertyBuilder::new()
        .family("Ubuntu")
        .build();
    let (font_data, _) = system_fonts::get(&font_property).unwrap();
    let font = Font::try_from_bytes(&font_data).unwrap();
    let scale = rusttype::Scale::uniform(15.0);
    let color = Rgba([0u8, 0u8, 0u8, 255u8]);
    let line_height = 15;

    let modified_image = imageproc::drawing::draw_text(
        image,
        color,
        0,
        2 * line_height,
        scale,
        &font,
        bbox.get_min_x().to_string().as_str(),
    );
    let modified_image = imageproc::drawing::draw_text(
        &modified_image,
        color,
        0,
        3 * line_height,
        scale,
        &font,
        bbox.get_max_x().to_string().as_str(),
    );
    let modified_image = imageproc::drawing::draw_text(
        &modified_image,
        color,
        0,
        4 * line_height,
        scale,
        &font,
        bbox.get_min_y().to_string().as_str(),
    );
    let modified_image = imageproc::drawing::draw_text(
        &modified_image,
        color,
        0,
        5 * line_height,
        scale,
        &font,
        bbox.get_max_y().to_string().as_str(),
    );
    let modified_image = imageproc::drawing::draw_text(
        &modified_image,
        color,
        0,
        6 * line_height,
        scale,
        &font,
        bbox.get_projection_code(),
    );

    *image = modified_image;
}

/// Configures logger
/// Is able to log to a http server, console and file
pub fn configure_logger() {
    let log_dir = env::var("LOG_DIR").unwrap_or({
        //get current directory:

        let log_dir = env::current_dir()
            .unwrap()
            .into_os_string()
            .into_string()
            .unwrap()
            + "/logs";

        //create dir if not exists:
        if !std::path::Path::new(&log_dir).exists() {
            fs::create_dir(&log_dir).unwrap();
        }

        log_dir
    });

    let now = Local::now();

    let log_file_location = format!("{}/log_{}-{}-{}.ansi.log", log_dir, now.year(), now.month(), now.day());

    let logfile = FileAppender::builder()
        .encoder(Box::new(PatternEncoder::new("{d} - {l} - {t} - {m};\n")))
        .build(log_file_location)
        .unwrap();

    let console_log = ConsoleAppender::builder()
        .encoder(Box::new(PatternEncoder::new("{d} - {l} - {t} - {m};\n")))
        .build();

    // let web_log = appender::WebAppender::builder()
    //     .encoder(Box::new(PatternEncoder::new("{d} - {l} - {t} - {m};\n")))
    //     .build("https://www.maris.nl/docker_log_debug.php").unwrap();

    let config = Config::builder()
        .appender(Appender::builder().build("logfile", Box::new(logfile)))
        .appender(Appender::builder().build("stdout", Box::new(console_log)))
        // .appender(Appender::builder().build("web_log", Box::new(web_log)))
        .build(
            Root::builder()
                .appender("logfile")
                .appender("stdout")
                //    .appender("web_log")
                .build(log::LevelFilter::Info),
        )
        .unwrap();

    log4rs::init_config(config).unwrap();

    log_panics::init(); 

    log::info!("Logger configured, logging to: {}", log_dir);
}

pub fn read_config_file() -> crate::config::ConfigFile {

    let config_dir = get_env_var("CONFIG_DIR", Some("../config"));
    let config_file_location = format!("{}/config.json", config_dir);
    let json_str =
        std::fs::read_to_string(config_file_location).expect("Failed to read config file");
    let mut parsed: crate::config::ConfigFile = serde_json::from_str(&json_str).unwrap();

    // redundant
    // if parsed.workspaces.is_some() {
    //     for mut workspace in parsed.workspaces.expect("") {
    //         for mut layer_config in workspace.layers {
    //             let mut layer_inner_config = layer_config.config;
    //             if layer_inner_config.dimensions.is_some() {
    //                 for (key, mut dimension) in layer_inner_config.dimensions.expect("").into_iter() {
                        
    //                     if dimension.accepted.is_some() {
    //                         match dimension.accepted.expect("") {
    //                             AcceptedValues::Multiple(values) => values,
    //                             AcceptedValues::Single(period) => {
    //                                 // return generate_iso_period_timestamps(period)
    //                             }
    //                         }
    //                     }
    //                 }
    //             }      
    //         }
    //     }    
    // }

    parsed
}

pub fn get_env_var(var_name: &str, default: Option<&str>) -> String {
    let default = match default {
        Some(d) => d,
        None => "",
    };
    env::var(var_name).unwrap_or_else(|_| default.to_string())
}

pub fn random_string(length: usize) -> String {
    let mut rng = rand::thread_rng();
    let mut random_string = String::new();

    for _ in 0..length {
        let random_char = rng.gen_range(0..=61);
        let char = match random_char {
            0..=25 => (random_char + 65) as u8 as char,
            26..=51 => (random_char + 71) as u8 as char,
            52..=61 => (random_char - 4) as u8 as char,
            _ => ' ',
        };

        random_string.push(char);
    }

    random_string
}

pub fn parse_range(input: &str) -> Result<(f64, f64), &str> {
    let parts: Vec<&str> = input.split(',').collect();

    if parts.len() != 2 {
        return Err("Too little elements in range, expected 2");
    }

    let start: f64 = parts[0]
        .trim()
        .parse()
        .map_err(|_| "Could not parse start value of range")?;
    let end: f64 = parts[1]
        .trim()
        .parse()
        .map_err(|_| "Could not parse end value of range")?;

    if start > end {
        return Err("Start value of range is greater than end value");
    }

    Ok((start, end))
}

pub fn get_layer_filepath(workspace_id: &str, layer_id: &str, hash: Option<&str>) -> Result<String, String> {
    let layer_directory = get_env_var(
        "LAYER_DIR",
        Some("../layers"),
    );

    // let hash = has the viewparams

    let hash = match hash {
        Some(h) => h.to_string(),
        None => "".to_string(),
    };

    // add a param for viewparams or query to hash and add to the filename.
    // also needs to be done for creating the layer
    let layer_filename = format!("{}_{}_{}.parquet", workspace_id, layer_id, hash);
    //let layer_filename = format!("{}_{}.parquet", workspace_id, layer_id);
    let mut layer_filepath = PathBuf::from(layer_directory);
    layer_filepath.push(layer_filename);

    layer_filepath
        .to_str()
        .map(|s| s.to_string())
        .ok_or_else(|| "Invalid layer filepath".to_string())

    // do here update layer if file_path is non existant?
}

pub fn hash_viewparams(viewparams: &HashMap<String, Value>) -> String {
    // Serialize to JSON string in a deterministic way
    let mut sorted: Vec<_> = viewparams.iter().collect();
    sorted.sort_by_key(|(k, _)| *k); // sort keys for deterministic hash
    let json_string = serde_json::to_string(&sorted).unwrap();

    // Compute SHA256 hash
    let mut hasher = Sha256::new();
    hasher.update(json_string.as_bytes());
    let result = hasher.finalize();

    // Return as hex string
    hex::encode(result)
}
