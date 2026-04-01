use std::collections::HashMap;
use serde_json::{Value};
use axum::http::StatusCode;
use chrono::{DateTime, Utc, Datelike, Duration};

use crate::{
    boundingbox::BoundingBox, color_maps::ColorMapsConfig, config::LayerConfig, map_querying::get_feature_info_collection::{Feature, GetFeatureInfoCollection}, query_parameters::{GetFeatureInfoRequestParameters, GetMapRequestParameters}, request_profiling::RequestProfiling
};

// ============================================
// Viewparams


pub fn parse_viewparams(viewparams: &Option<String>) -> HashMap<String, Value> {
     match viewparams {
        Some(vp_str) => {
            let mut vp_map = HashMap::<String, Value>::new();

            for pair in vp_str.split(';') {
                let mut iter = pair.splitn(2, ':');
                if let (Some(key), Some(value_str)) = (iter.next(), iter.next()) {
                    // Attempt to parse the value as JSON
                    // This will handle numbers, arrays, strings
                    let parsed_value: Value = serde_json::from_str(value_str)
                        // fallback to string if parsing fails
                        .unwrap_or(Value::String(value_str.to_string()));
                    vp_map.insert(key.to_string(), parsed_value);
                }
            }
            vp_map
        }
        None => HashMap::<String, Value>::new(),
    }
}

// pub async fn assign_viewparams_in_config(
//     layer_configs: &mut Vec<crate::config::LayerConfig>,
//     viewparams: &HashMap<String, Value>,
// ) -> Result<(), (StatusCode, String)> {

//     for layer_config in layer_configs {
//         // Ensure assigned_viewparams exists (with defaults if present)
//         let assigned_viewparams = layer_config
//             .config
//             .assigned_viewparams
//             .get_or_insert_with(HashMap::new);

//         // If there are allowed viewparams, validate & assign requested ones
//         if let Some(allowed) = &layer_config.config.available_viewparams {
//             if let Err((status, msg)) = self::assign_viewparams(
//                 allowed,
//                 assigned_viewparams,       // mutable reference to defaults
//                 viewparams      // user input to overwrite defaults
//             ).await {
//                 return Err((status, msg));  // stop immediately on invalid input
//             }
//         }
//         // assigned_viewparams now contains defaults + any valid input
//     }

//     Ok(())
// }

pub async fn assign_viewparams_in_config(
    layer_config: &mut LayerConfig,
    viewparams: &HashMap<String, Value>,
) -> Result<(), (StatusCode, String)> {

    // Ensure assigned_viewparams exists (with defaults if present)
    let assigned_viewparams = layer_config
        .config
        .assigned_viewparams
        .get_or_insert_with(HashMap::new);

    // If there are allowed viewparams, validate & assign requested ones
    if let Some(allowed) = &layer_config.config.available_viewparams {
        if let Err((status, msg)) = self::assign_viewparams(
            allowed,
            assigned_viewparams,       // mutable reference to defaults
            viewparams      // user input to overwrite defaults
        ).await {
            return Err((status, msg));  // stop immediately on invalid input
        }
    }
    // assigned_viewparams now contains defaults + any valid input

    Ok(())
}

/**
 * Resolves and validates viewparams from the request against the allowed parameters defined in the config.
 * Only parameters that are defined in the config and have valid the type will they be included in the returned HashMap.
 * Else throw error
 */
pub async fn assign_viewparams(
    allowed: &HashMap<String, Value>,
    assigned: &mut HashMap<String, Value>,
    input: &HashMap<String, Value>,
) -> Result<(), (StatusCode, String)> {

    for (param, value) in input {
        // Parameter must exist in allowed config
        let expected = match allowed.get(param) {
            Some(v) => v,
            None => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("Param '{}' not allowed", param),
                ));
            }
        };

        let valid = match expected.get("type") {
            // can we not define t once?
            Some(Value::String(t)) if t == "numeric" => value.is_number(),
            Some(Value::String(t)) if t == "string" => value.is_string(),
            Some(Value::String(t)) if t == "bool" => value.is_boolean(),
            Some(Value::String(t)) if t == "numeric_array" => {
                value.as_array()
                    .map(|arr| arr.iter().all(|v| v.is_number()))
                    .unwrap_or(false)
            },
            Some(Value::String(t)) if t == "string_array" => {
                value.as_array()
                    .map(|arr| arr.iter().all(|v| v.is_string()))
                    .unwrap_or(false)
            },
            _ => false,
        };

        if !valid {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Type mismatch for param '{}'", param),
            ));
        }

        // depth exception checking on allowed depths
        if param == "depth" {
            if let Some(allowed_bins) = expected.get("allowed") {
                let arr = value.as_array().unwrap();
                let min = arr[0].as_f64().unwrap() as i32;
                let max = arr[1].as_f64().unwrap() as i32;
            
                let valid_bin = allowed_bins.as_object()
                    .map(|bins| bins.values().any(|v| {
                        let v_arr = v.as_array().unwrap();
                        let v_min = v_arr[0].as_f64().unwrap() as i32;
                        let v_max = v_arr[1].as_f64().unwrap() as i32;
                        min == v_min && max == v_max
                    }))
                    .unwrap_or(false);
                
                if !valid_bin {
                    return Err((StatusCode::BAD_REQUEST,
                        format!("Depth range [{},{}] not allowed for this layer", min, max)));
                }
            }
        }

        // Assign the value, overwriting default if present
        assigned.insert(param.clone(), value.clone());
    }

    Ok(())
}


pub fn apply_viewparams_to_query(
    mut query_str: String,
    assigned_viewparams: Option<&HashMap<String, Value>>,
) -> String {

    // Convert query JSON to string
    let params = match assigned_viewparams {
        Some(p) if !p.is_empty() => p,
        _ => return query_str,
    };

    println!("original query string {}", query_str);

    for (key, value) in params {
        let key = key.to_lowercase();
        match value {
            // ---- STRING ----
            Value::String(s) => {
                // Only replace unquoted placeholder
                let placeholder = format!("%{}%", key);
                query_str = query_str.replace(&placeholder, s);
            }

            // ---- NUMBER / BOOL ----
            Value::Number(_) | Value::Bool(_) => {
                let replacement = value.to_string();

                // Replace quoted first
                let quoted_placeholder = format!("\"%{}%\"", key);
                query_str = query_str.replace(&quoted_placeholder, &replacement);

                // Then unquoted
                let placeholder = format!("%{}%", key);
                query_str = query_str.replace(&placeholder, &replacement);
            }

            // ---- ARRAY ----
            Value::Array(arr) => {
                for (i, elem) in arr.iter().enumerate() {
                    match elem {
                        Value::String(s) => {
                            let placeholder = format!("%{}[{}]%", key, i);
                            query_str = query_str.replace(&placeholder, s);
                        }

                        Value::Number(_) | Value::Bool(_) => {
                            let replacement = elem.to_string();

                            let quoted_placeholder =
                                format!("\"%{}[{}]%\"", key, i);
                            query_str =
                                query_str.replace(&quoted_placeholder, &replacement);

                            let placeholder = format!("%{}[{}]%", key, i);
                            query_str = query_str.replace(&placeholder, &replacement);
                        }

                        _ => {
                            log::warn!(
                                "Unhandled array element type for key {}[{}]",
                                key,
                                i
                            );
                        }
                    }
                }
            }

            _ => {
                log::warn!("Unhandled viewparam type for key {}", key);
            }
        }
    }

    println!("applied query string {}", query_str);

    query_str
}



// TODO:

// for viewparams
// parse viewparams can remain the same only change:
// // put the hasmap into another hashmap named queryparams

// for ogc params / dimension params
// single function that takes both time and elevation
// returns hashmap dimensions{}
// if time is empty string don't add time to hashmap
// if time is invalid return error (only accept single time values)
// if elevation is empty string don't add to hashmap

// ==================================================
// Dimensions

use regex::Regex;

/// Strict ISO 8601: YYYY-MM-DDThh:mm:ssZ
fn parse_ogc_time(time: &str) -> Result<String, String> {
    let re = Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")
        .map_err(|_| "Internal regex error".to_string())?;

    if re.is_match(time) {
        Ok(time.to_string())
    } else {
        Err("Invalid time format. Expected YYYY-MM-DDThh:mm:ssZ".to_string())
    }
}

/// Elevation: "min/max"
fn parse_ogc_elevation(elevation: &str) -> Result<String, String> {
    // ---- reject invalid multi-range input ----
    if elevation.contains(',') {
        return Err("Invalid character ',' or multiple ranges not allowed".to_string());
    }

    // ---- split min/max ----
    let parts: Vec<&str> = elevation.split('/').collect();
    if parts.len() != 2 {
        return Err("Invalid length expected min/max".to_string());
    }

    // ---- parse numeric values ----
    let min = parts[0].trim().parse::<f64>()
        .map_err(|_| "Invalid elevation value: expected number".to_string())?;

    let max = parts[1].trim().parse::<f64>()
        .map_err(|_| "Invalid elevation value: expected number".to_string())?;

    // ---- ordering check (raw values) ----
    if min > max {
        return Err("Invalid order of elevation, expected min/max".to_string());
    }

    // ---- sign consistency check ----
    let min_neg = min < 0.0;
    let max_neg = max < 0.0;

    if min_neg != max_neg {
        return Err("Invalid range given, expected all negative or positive values".to_string());
    }

    // ---- normalise output (absolute values) ----
    let a = min.abs();
    let b = max.abs();

    let min_abs = a.min(b);
    let max_abs = a.max(b);

    Ok(format!("{}/{}", min_abs, max_abs))
}

/// Main function (same style as parse_viewparams)
pub fn parse_time_elevation(
    time: &Option<String>,
    elevation: &Option<String>,
) -> Result<HashMap<String, Value>, String> {
    let mut map = HashMap::<String, Value>::new();

    // ---- TIME ----
    if let Some(t) = time {
        match parse_ogc_time(t) {
            Ok(valid) => {
                map.insert("time".to_string(), Value::String(valid));
            }
            Err(e) => return Err(e),
        }
    }

    // ---- ELEVATION ----
    if let Some(e) = elevation {
        match parse_ogc_elevation(e) {
            Ok(valid) => {
                map.insert("elevation".to_string(), Value::String(valid));
            }
            Err(e) => return Err(e),
        }
    }

    Ok(map)
}

// check if elevation is accepted by layer
pub fn check_accepted_elevations(
    requested_elevation: &str,
    layer_dimensions: &HashMap<String, Value>,
    layername: &str,
) -> Result<Vec<Value>, String> {
    // Check if elevation exists in config
    let elevation = layer_dimensions
        .get("elevation")
        .ok_or_else(|| "elevation dimension not allowed".to_string())?;

    let elevation_obj = elevation
        .as_object()
        .ok_or_else(|| "elevation dimension must be an object".to_string())?;

    // Check accepted is array
    let accepted = elevation_obj
        .get("accepted")
        .ok_or_else(|| "elevation dimension not allowed".to_string())?
        .as_array()
        .ok_or_else(|| "elevation dimension in config must be an array".to_string())?;

    // Validate requested value is in accepted list
    let is_valid = accepted.iter().any(|v| {
        v.as_str()
            .map(|s| s.trim() == requested_elevation.trim())
            .unwrap_or(false)
    });

    if !is_valid {
        return Err(format!("requested elevation not allowed for this layer {}", layername));
    }

    // Parse "min-max"
    let parts: Vec<&str> = requested_elevation.split('/').collect();
    if parts.len() != 2 {
        return Err("invalid elevation format".to_string());
    }

    let min: f64 = parts[0]
        .trim()
        .parse()
        .map_err(|_| "invalid elevation format".to_string())?;

    let max: f64 = parts[1]
        .trim()
        .parse()
        .map_err(|_| "invalid elevation format".to_string())?;

    Ok(vec![Value::from(min), Value::from(max)])
}


// check if time dimension is accepted by layer
pub fn check_accepted_times(
    requested_time: &str,
    layer_dimensions: &HashMap<String, Value>,
    layername: &str,
) -> Result<HashMap<String, Value>, String> {

    //===============================
    // layer dimension

    // check if time dimension is inside the layer config dimensions and if format in the layer config is correct
    let time = layer_dimensions
        .get("time")
        .ok_or_else(|| "time dimension not allowed".to_string())?;
    let time_obj = time
        .as_object()
        .ok_or_else(|| "time dimension must be an object".to_string())?;
    let accepted = time_obj
        .get("accepted")
        .ok_or_else(|| "time dimension not allowed".to_string())?
        .as_str()
        .ok_or_else(|| "time dimension in config must be a string".to_string())?;

    let parts: Vec<&str> = accepted.split('/').collect();
    if parts.len() != 3 {
        return Err(format!(
            "time format in layer {} does not conform to Rn/YYYY-MM-DDThh:mm:ssZ/interval",
            layername
        ));
    }

    // Enforce Rn (no infinite allowed)
    let repeat_str = parts[0];
    if !repeat_str.starts_with('R') || repeat_str.len() <= 1 {
        return Err(format!(
            "time format in layer {} does not conform to Rn/YYYY-MM-DDThh:mm:ssZ/interval",
            layername
        ));
    }

    let repeat: u32 = repeat_str[1..]
        .parse()
        .map_err(|_| format!(
            "time format in layer {} does not conform to Rn/YYYY-MM-DDThh:mm:ssZ/interval",
            layername
        ))?;

    if repeat == 0 {
        return Err(format!(
            "time format in layer {} does not conform to Rn/YYYY-MM-DDThh:mm:ssZ/interval",
            layername
        ));
    }

    let start: DateTime<Utc> = parts[1]
        .parse()
        .map_err(|_| format!(
            "time format in layer {} does not conform to Rn/YYYY-MM-DDThh:mm:ssZ/interval",
            layername
        ))?;

    let period_str = parts[2];

    let unit = match period_str {
        "P1Y" => "year",
        "P1M" => "month",
        "P1D" => "day",
        _ => {
            return Err(format!(
                "time format in layer {} does not conform to Rn/YYYY-MM-DDThh:mm:ssZ/interval",
                layername
            ))
        }
    };

    //==================================
    // requested time

    // parse requested time
    let requested: DateTime<Utc> = requested_time
        .parse()
        .map_err(|_| "invalid requested time format".to_string())?;

    // must requested time must atleast be greater equel than dimension accepted start of range
    if requested < start {
        return Err(format!("date not accepted for layer {}", layername));
    }

    let mut current = start;

    for _ in 0..repeat {
        if current == requested {
            break;
        }

        current = match unit {
            "year" => current.with_year(current.year() + 1).ok_or("invalid date")?,
            "month" => {
                let mut y = current.year();
                let mut m = current.month() as i32 + 1;

                if m > 12 {
                    y += 1;
                    m -= 12;
                }

                current
                    .with_year(y)
                    .and_then(|d| d.with_month(m as u32))
                    .ok_or("invalid date")?
            }
            "day" => current + Duration::days(1),
            _ => unreachable!(),
        };

        if current == requested {
            break;
        }
    }

    if current != requested {
        return Err(format!("date not accepted for layer {}", layername));
    }

    let mut result = HashMap::new();

    match unit {
        "year" => {
            result.insert("year".to_string(), Value::from(requested.year()));
        }
        "month" => {
            result.insert("year".to_string(), Value::from(requested.year()));
            result.insert("month".to_string(), Value::from(requested.month()));
        }
        "day" => {
            result.insert("year".to_string(), Value::from(requested.year()));
            result.insert("month".to_string(), Value::from(requested.month()));
            result.insert("day".to_string(), Value::from(requested.day()));
        }
        _ => {}
    }

    Ok(result)
}


pub fn apply_dimensions_to_viewparams(
    requested_viewparams: &HashMap<String, Value>,
    requested_dimensions: &HashMap<String, Value>,
    layer_dimensions: &Option<HashMap<String, Value>>,
    layername: &str,
) -> Result<HashMap<String, Value>, String> {
    // 1. Extract layer_dimensions or fallback to empty map
    let empty_map = HashMap::new();
    let layer_dims = layer_dimensions.as_ref().unwrap_or(&empty_map);

    // 2. Both empty → return original viewparams
    if layer_dims.is_empty() && requested_dimensions.is_empty() {
        return Ok(requested_viewparams.clone());
    }

    // 3. Layer has no dimensions but request does → error
    if layer_dims.is_empty() && !requested_dimensions.is_empty() {
        return Err(format!("layer {} does not accept any dimensions", layername));
    }

    // 4. Both non-empty → apply logic
    let mut result = requested_viewparams.clone();

    // Elevation
    if let Some(elevation_value) = requested_dimensions.get("elevation") {
        let elevation_str = elevation_value
            .as_str()
            .ok_or_else(|| "elevation must be a string".to_string())?;

        let parsed = check_accepted_elevations(
            elevation_str,
            layer_dims,
            layername,
        )?;

        result.insert("depth".to_string(), Value::Array(parsed));
    }

    // Time
    if let Some(time_value) = requested_dimensions.get("time") {
        let time_str = time_value
            .as_str()
            .ok_or_else(|| "time must be a string".to_string())?;

        let parsed_time =
            check_accepted_times(time_str, layer_dims, layername)?;

        for (k, v) in parsed_time {
            result.insert(k, v);
        }
    }

    Ok(result)
}