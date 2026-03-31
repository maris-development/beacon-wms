use std::collections::HashMap;

// use reqwest::StatusCode;
// use std::str::FromStr;

// use chrono::{DateTime, Utc, Datelike, NaiveDate, Duration};
use serde_json::{Value, json};
use axum::response::{IntoResponse, Response};
use axum::http::StatusCode;



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
    layer_configs: &mut Vec<crate::config::LayerConfig>,
    viewparams: &HashMap<String, Value>,
) -> Result<(), (StatusCode, String)> {

    for layer_config in layer_configs {
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
    }

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

    // EDIT: we should also add the allowed depths here and check them

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
                let min = arr[0].as_i64().unwrap() as i32;
                let max = arr[1].as_i64().unwrap() as i32;
            
                let valid_bin = allowed_bins.as_object()
                    .map(|bins| bins.values().any(|v| {
                        let v_arr = v.as_array().unwrap();
                        let v_min = v_arr[0].as_i64().unwrap() as i32;
                        let v_max = v_arr[1].as_i64().unwrap() as i32;
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