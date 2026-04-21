use std::collections::HashMap;
use serde_json::{Value};
use axum::http::StatusCode;
use chrono::{DateTime, Utc, Datelike, Duration};

use crate::{
     config::LayerConfig
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

    println!("Applied viewparams to query: {}", query_str);

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
/// This currently does not accept values with ms precision or timezone offsets
fn parse_ogc_time(time: &str) -> Result<String, String> {
    // let re = Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")
    //     .map_err(|_| "Internal regex error".to_string())?;

    // with optional ms precision
    // not timezone offset because we don't want that
    let re = Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z$")
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

    // Parse PnY / PnM / PnD where n is any positive integer
    let period_re = Regex::new(r"^P(\d+)(Y|M|D)$")
        .map_err(|_| "Internal regex error".to_string())?;

    let (step, unit) = match period_re.captures(period_str) {
        Some(caps) => {
            let n: u32 = caps[1].parse().map_err(|_| format!(
                "time format in layer {} does not conform to Rn/YYYY-MM-DDThh:mm:ssZ/interval",
                layername
            ))?;
            if n == 0 {
                return Err(format!(
                    "time format in layer {} does not conform to Rn/YYYY-MM-DDThh:mm:ssZ/interval",
                    layername
                ));
            }
            let unit = match &caps[2] {
                "Y" => "year",
                "M" => "month",
                "D" => "day",
                _ => unreachable!(),
            };
            (n, unit)
        }
        None => return Err(format!(
            "time format in layer {} does not conform to Rn/YYYY-MM-DDThh:mm:ssZ/interval",
            layername
        )),
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
            "year" => current
                .with_year(current.year() + step as i32)
                .ok_or("invalid date")?,
            "month" => {
                let mut y = current.year();
                let mut m = current.month() as i32 + step as i32;

                while m > 12 {
                    y += 1;
                    m -= 12;
                }

                current
                    .with_year(y)
                    .and_then(|d| d.with_month(m as u32))
                    .ok_or("invalid date")?
            }
            "day" => current + Duration::days(step as i64),
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
            if step == 1 {
                result.insert("year".to_string(), Value::from(requested.year()));
            } else {
                result.insert("year_from".to_string(), Value::from(requested.year()));
                result.insert("year_to".to_string(), Value::from(requested.year() + step as i32 - 1));
            }
        }
        "month" => {
            let year = requested.year();
            let month = requested.month();

            // compute next month
            let (next_year, next_month) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };

            // first day of next month
            let first_next_month = requested
                .with_year(next_year)
                .and_then(|d| d.with_month(next_month))
                .and_then(|d| d.with_day(1))
                .ok_or("invalid date")?;

            // last day of current month
            let last_day = (first_next_month - Duration::days(1)).day();

            result.insert("year".to_string(), Value::from(year));
            result.insert("month".to_string(), Value::from(month));
            result.insert("day".to_string(), Value::from(last_day));
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



#[cfg(test)]
mod test_parse_dimensions {
    use super::*;
    use serde_json::{Value};
    use std::collections::HashMap;

    #[test]
    fn test_both_valid() {
        let time = Some("2024-01-01T12:00:00Z".to_string());
        let elevation = Some("0/10".to_string());

        let result = parse_time_elevation(&time, &elevation).unwrap();

        let mut expected = HashMap::<String, Value>::new();
        expected.insert("time".to_string(), Value::String("2024-01-01T12:00:00Z".to_string()));
        expected.insert("elevation".to_string(), Value::String("0/10".to_string()));

        println!("parsed dimensions: {:?}", result);

        assert_eq!(result, expected);
    }

    #[test]
    fn test_only_time_valid() {
        let time = Some("2024-01-01T12:00:00Z".to_string());
        let elevation = None;

        let result = parse_time_elevation(&time, &elevation).unwrap();

        let mut expected = HashMap::<String, Value>::new();
        expected.insert("time".to_string(), Value::String("2024-01-01T12:00:00Z".to_string()));
        println!("parsed dimensions: {:?}", result);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_only_elevation_valid() {
        let time = None;
        let elevation = Some("0/10".to_string());

        let result = parse_time_elevation(&time, &elevation).unwrap();

        let mut expected = HashMap::<String, Value>::new();
        expected.insert("elevation".to_string(), Value::String("0/10".to_string()));
        println!("parsed dimensions: {:?}", result);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_empty_inputs() {
        let time: Option<String> = None;
        let elevation: Option<String> = None;

        let result = parse_time_elevation(&time, &elevation).unwrap();

        let expected = HashMap::<String, Value>::new();
        println!("parsed dimensions: {:?}", result);
        assert_eq!(result, expected);
    }

    #[test]
    fn test_invalid_time_returns_error() {
        let time = Some("2024-01-01".to_string()); // invalid
        let elevation = None;

        let result = parse_time_elevation(&time, &elevation);
        println!("parsed dimensions: {:?}", result);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Invalid time format. Expected YYYY-MM-DDThh:mm:ssZ"
        );
    }

    #[test]
    fn test_invalid_elevation_returns_error() {
        let time = None;
        let elevation = Some("10/0".to_string()); // invalid ordering

        let result = parse_time_elevation(&time, &elevation);
        println!("parsed dimensions: {:?}", result);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Invalid order of elevation, expected min/max"
        );
    }

    #[test]
    fn test_invalid_time_short_circuits() {
        let time = Some("bad".to_string());
        let elevation = Some("0/10".to_string());

        let result = parse_time_elevation(&time, &elevation);
        println!("parsed dimensions: {:?}", result);
        // should fail fast on time, elevation ignored
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_elevation_short_circuits() {
        let time = Some("2024-01-01T12:00:00Z".to_string());
        let elevation = Some("a/b".to_string());

        let result = parse_time_elevation(&time, &elevation);
        println!("parsed dimensions: {:?}", result);
        assert!(result.is_err());
    }

    // negative elevation values
    #[test]
    fn test_valid_negative_elevation() {
        let time = None;
        let elevation = Some("-20/-10".to_string()); // invalid ordering

        let result = parse_time_elevation(&time, &elevation).unwrap();

        let mut expected = HashMap::<String, Value>::new();
        expected.insert("elevation".to_string(), Value::String("10/20".to_string()));
        println!("parsed dimensions: {:?}", result);
        assert_eq!(result, expected);
    }
}

// =========================================================================
// check dimensions

#[cfg(test)]
mod test_check_dimensions {
    use super::*;
    use serde_json::{json};
    use std::collections::HashMap;

    fn sample_layer_dimensions() -> HashMap<String, serde_json::Value> {
        serde_json::from_value(json!({
            "time": {
                "default": "2021-01-01T00:00:00Z",
                "units": "ISO8601",
                "accepted": "R500/1950-01-01T00:00:00Z/P1Y"
            },
            "elevation": {
                "default": "0-5",
                "units": "m",
                "viewparam": "depth",
                "accepted": [
                    "0-5",
                    "5-10",
                    "10-20",
                    "20-30",
                    "30-50",
                    "50-75",
                    "75-100",
                    "100-125",
                    "125-150",
                    "150-200",
                    "200-250",
                    "250-300",
                    "300-400",
                    "400-500",
                    "500-600",
                    "600-700",
                    "700-800",
                    "800-900",
                    "900-1000",
                    "1000-1100",
                    "1100-1200",
                    "1200-1300",
                    "1300-1400",
                    "1400-1500",
                    "1500-1750",
                    "1750-2000",
                    "2000-2500",
                    "2500-3000",
                    "3000-3500",
                    "3500-4000",
                    "4000-4500",
                    "4500-5000",
                    "5000-12000"
                ]
            }
        }))
        .unwrap()
    }

    // elevation
    #[test]
    fn test_valid_elevation_returns_range() {
        let dims = sample_layer_dimensions();

        let result = check_accepted_elevations(
            "5-10",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_ok());

        let range = result.unwrap();
        assert_eq!(range.len(), 2);
    }

    #[test]
    fn test_invalid_elevation_not_allowed() {
        let dims = sample_layer_dimensions();

        let result = check_accepted_elevations(
            "9999-10000",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "requested elevation not allowed for this layer testlayer"
        );
    }

    #[test]
    fn test_missing_elevation_dimension() {
        let mut dims = sample_layer_dimensions();
        dims.remove("elevation");

        let result = check_accepted_elevations(
            "0-5",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "elevation dimension not allowed"
        );
    }

    #[test]
    fn test_accepted_not_array() {
        let mut dims = sample_layer_dimensions();

        dims.get_mut("elevation").map(|e| {
            if let Some(obj) = e.as_object_mut() {
                obj.insert(
                    "accepted".to_string(),
                    serde_json::Value::String("0-5".to_string()),
                );
            }
        });

        let result = check_accepted_elevations(
            "0-5",
            &dims,
            "testlayer",
        );
        println!("{:?}", result);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "elevation dimension in config must be an array"
        );
    }

    // time
    #[test]
    fn test_valid_time_start_of_range() {
        let dims = sample_layer_dimensions();

        let result = check_accepted_times(
            "1950-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_ok());

        let map = result.unwrap();
        assert_eq!(map.get("year").unwrap(), &serde_json::json!(1950));
    }

    #[test]
    fn test_valid_time_within_range() {
        let dims = sample_layer_dimensions();

        let result = check_accepted_times(
            "1955-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_ok());

        let map = result.unwrap();
        assert_eq!(map.get("year").unwrap(), &serde_json::json!(1955));
    }

    #[test]
    fn test_time_outside_range() {
        let dims = sample_layer_dimensions();

        let result = check_accepted_times(
            "2050-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "date not accepted for layer testlayer"
        );
    }

    #[test]
    fn test_missing_time_dimension() {
        let mut dims = sample_layer_dimensions();
        dims.remove("time");

        let result = check_accepted_times(
            "1950-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "time dimension not allowed"
        );
    }

    #[test]
    fn test_invalid_time_format() {
        let dims = sample_layer_dimensions();

        let result = check_accepted_times(
            "1950/01/01",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_err());
    }

    #[test]
    fn test_time_boundary_last_valid_year() {
        let dims = sample_layer_dimensions();

        let result = check_accepted_times(
            "1999-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);

        assert!(result.is_ok());

        let map = result.unwrap();
        assert_eq!(map.get("year").unwrap(), &serde_json::json!(1999));
    }

    fn sample_layer_dimensions_30y() -> HashMap<String, serde_json::Value> {
        // R2 = 2 advances from start → valid positions: 1950, 1980, 2010
        serde_json::from_value(json!({
            "time": {
                "default": "1950-01-01T00:00:00Z",
                "units": "ISO8601",
                "accepted": "R2/1950-01-01T00:00:00Z/P30Y"
            }
        }))
        .unwrap()
    }

    #[test]
    fn test_p30y_first_period_returns_year_from_to() {
        let dims = sample_layer_dimensions_30y();

        let result = check_accepted_times(
            "1950-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);
        assert!(result.is_ok());

        let map = result.unwrap();
        assert_eq!(map.get("year_from").unwrap(), &serde_json::json!(1950));
        assert_eq!(map.get("year_to").unwrap(), &serde_json::json!(1979));
        assert!(map.get("year").is_none());
    }

    #[test]
    fn test_p30y_second_period_returns_year_from_to() {
        let dims = sample_layer_dimensions_30y();

        let result = check_accepted_times(
            "1980-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);
        assert!(result.is_ok());

        let map = result.unwrap();
        assert_eq!(map.get("year_from").unwrap(), &serde_json::json!(1980));
        assert_eq!(map.get("year_to").unwrap(), &serde_json::json!(2009));
    }

    #[test]
    fn test_p30y_third_period_returns_year_from_to() {
        let dims = sample_layer_dimensions_30y();

        let result = check_accepted_times(
            "2010-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);
        assert!(result.is_ok());

        let map = result.unwrap();
        assert_eq!(map.get("year_from").unwrap(), &serde_json::json!(2010));
        assert_eq!(map.get("year_to").unwrap(), &serde_json::json!(2039));
    }

    #[test]
    fn test_p30y_non_boundary_date_rejected() {
        let dims = sample_layer_dimensions_30y();

        // 1955 is not a valid 30-year boundary starting from 1950
        let result = check_accepted_times(
            "1955-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "date not accepted for layer testlayer");
    }

    #[test]
    fn test_p30y_out_of_range_rejected() {
        let dims = sample_layer_dimensions_30y();

        // R3 gives 3 periods: 1950, 1980, 2010 — 2040 is one beyond
        let result = check_accepted_times(
            "2040-01-01T00:00:00Z",
            &dims,
            "testlayer",
        );

        println!("{:?}", result);
        assert!(result.is_err());
    }

}


