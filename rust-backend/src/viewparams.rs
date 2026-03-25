use std::collections::HashMap;

use reqwest::StatusCode;
use serde_json::Value;

use std::str::FromStr;

use chrono::{DateTime, Utc, Datelike, NaiveDate, Duration};


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


#[derive(Debug, Clone)]
pub enum DimensionValue<T> {
    Single(T),
    List(Vec<T>),
    Range {
        start: T,
        end: T,
        step: Option<T>,
    },
}

#[derive(Debug)]
pub enum ParseError {
    InvalidFormat,
    InvalidNumber,
}

// REMOVE
/// Generic parser for elevation (numeric)
pub fn parse_numeric_dimension(
    input: Option<String>,
) -> Result<Option<DimensionValue<f64>>, ParseError> {
    match input {
        None => Ok(None),
        Some(s) => {
            let s = s.trim();

            if s.contains('/') {
                // Range
                let parts: Vec<&str> = s.split('/').collect();
                if parts.len() < 2 || parts.len() > 3 {
                    return Err(ParseError::InvalidFormat);
                }

                let start = f64::from_str(parts[0]).map_err(|_| ParseError::InvalidNumber)?;
                let end = f64::from_str(parts[1]).map_err(|_| ParseError::InvalidNumber)?;
                let step = if parts.len() == 3 {
                    Some(f64::from_str(parts[2]).map_err(|_| ParseError::InvalidNumber)?)
                } else {
                    None
                };

                Ok(Some(DimensionValue::Range { start, end, step }))
            } else if s.contains(',') {
                // List
                let values = s
                    .split(',')
                    .map(|v| f64::from_str(v.trim()).map_err(|_| ParseError::InvalidNumber))
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(Some(DimensionValue::List(values)))
            } else {
                // Single
                let value = f64::from_str(s).map_err(|_| ParseError::InvalidNumber)?;
                Ok(Some(DimensionValue::Single(value)))
            }
        }
    }
}

// REMOVE
/// Parser for time (kept as String, but structured)
pub fn parse_time_dimension(
    input: Option<String>,
) -> Result<Option<DimensionValue<String>>, ParseError> {
    match input {
        None => Ok(None),
        Some(s) => {
            let s = s.trim();

            if s.contains('/') {
                let parts: Vec<&str> = s.split('/').collect();
                if parts.len() < 2 || parts.len() > 3 {
                    return Err(ParseError::InvalidFormat);
                }

                let start = parts[0].to_string();
                let end = parts[1].to_string();
                let step = if parts.len() == 3 {
                    Some(parts[2].to_string())
                } else {
                    None
                };

                Ok(Some(DimensionValue::Range { start, end, step }))
            } else if s.contains(',') {
                let values = s.split(',').map(|v| v.trim().to_string()).collect();

                Ok(Some(DimensionValue::List(values)))
            } else {
                Ok(Some(DimensionValue::Single(s.to_string())))
            }
        }
    }
}


pub fn parse_ogc_elevation(
    input: &Option<String>,
) -> Result<HashMap<String, Value>, String> {
    let input = match input.as_deref() {
        Some(s) => s.trim(),
        None => return Err("missing input".into()),
    };
    let input = input.trim();

    // extract numeric values
    let (start, end) = if input.contains('/') {
        // allow optional step → ignore everything after second value
        let parts: Vec<&str> = input.split('/').collect();
        if parts.len() < 2 {
            return Err("invalid elevation format".into());
        }
        split_ogc_elevation(parts[0], parts[1])?
    } else if input.contains(',') {
        let parts: Vec<&str> = input.split(',').collect();
        if parts.len() != 2 {
            return Err("invalid elevation format".into());
        }
        split_ogc_elevation(parts[0], parts[1])?
    } else {
        return Err("invalid elevation format".into());
    };

    // values as absolute and in correct order
    let min = start.abs().min(end.abs());
    let max = start.abs().max(end.abs());

    // build result
    let mut result = HashMap::new();
    result.insert("depth".to_string(), Value::Array(vec![Value::from(min), Value::from(max)]));

    Ok(result)
}


/// Helper: parse two string parts into f64 tuple
fn split_ogc_elevation(a: &str, b: &str) -> Result<(f64, f64), String> {
    let start: f64 = a.trim().parse().map_err(|_| "invalid number")?;
    let end: f64 = b.trim().parse().map_err(|_| "invalid number")?;
    Ok((start, end))
}

// parse ogc time string iso 8601 format
// accepted YYYY-MM-DDThh:mm:ssZ/PnX
pub fn parse_ogc_time(
    input: &Option<String>,
) -> Result<HashMap<String, Value>, String> {
    let raw_input = match input.as_deref() {
        Some(s) => s.trim(),
        None => return Err("missing input".into()),
    };

    let input = raw_input.trim();

    // Must contain duration
    let parts: Vec<&str> = input.split('/').collect();
    if parts.len() != 2 {
        return Err(format!(
            "invalid input '{}': must contain a single date and a duration like /P1Y, /P1M, /P1W, /P1D",
            input
        ));
    }

    let date_str = parts[0];
    let duration_str = parts[1];

    // Parse date
    let dt = DateTime::parse_from_rfc3339(date_str)
        .map_err(|_| format!("invalid date '{}': expected YYYY-MM-DDThh:mm:ssZ", date_str))?
        .with_timezone(&Utc);

    let mut result = HashMap::new();

    match duration_str {
        "P1Y" => {
            result.insert("year".to_string(), Value::from(format!("{:04}", dt.year())));
        }
        "P1M" => {
            let year = dt.year();
            let month = dt.month();

            let first_day = format!("{:04}-{:02}-01", year, month);

            let (next_year, next_month) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };

            let first_of_next_month = NaiveDate::from_ymd_opt(next_year, next_month, 1)
                .ok_or("invalid date computing last day of month")?;

            let last_day = (first_of_next_month - Duration::days(1))
                .format("%Y-%m-%d")
                .to_string();

            result.insert(
                "month".to_string(),
                Value::Array(vec![Value::from(first_day), Value::from(last_day)]),
            );
        }
        "P1W" => {
            // ISO week: Monday as start
            let weekday = dt.weekday().num_days_from_monday();
            let start_of_week = dt.date_naive() - Duration::days(weekday.into());
            let end_of_week = start_of_week + Duration::days(6);

            result.insert(
                "week".to_string(),
                Value::Array(vec![
                    Value::from(start_of_week.format("%Y-%m-%d").to_string()),
                    Value::from(end_of_week.format("%Y-%m-%d").to_string()),
                ]),
            );
        }
        "P1D" => {
            let day_str = format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day());
            result.insert("day".to_string(), Value::from(day_str));
        }
        _ => {
            return Err(format!(
                "invalid duration '{}': expected one of P1Y, P1M, P1W, P1D",
                duration_str
            ));
        }
    }

    Ok(result)
}

/// Applies OGC elevation and time values to viewparams
/// elevation and time are optional; viewparams is updated in-place
pub fn ogc_to_viewparams(
    mut viewparams: HashMap<String, Value>,
    elevation: &Option<String>,
    time: &Option<String>,
) -> Result<HashMap<String, Value>, String> {
    // --- Elevation ---
    if elevation.is_some() {
        let elev_map = parse_ogc_elevation(elevation)?;
        viewparams.extend(elev_map);
    }

    // --- Time ---
    if time.is_some() {
        let time_map = parse_ogc_time(time)?;
        viewparams.extend(time_map);
    }

    Ok(viewparams)
}