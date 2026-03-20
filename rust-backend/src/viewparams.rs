use std::collections::HashMap;

use reqwest::StatusCode;
use serde_json::Value;


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

        // Validate type
        let valid = match expected {
            Value::String(t) if t == "numeric" => value.is_number(),
            Value::String(t) if t == "string" => value.is_string(),
            Value::String(t) if t == "bool" => value.is_boolean(),
            Value::Array(expected_arr) => {
                if let Some(arr) = value.as_array() {
                    arr.len() == expected_arr.len()
                        && arr.iter().all(|v| v.is_number())
                } else {
                    false
                }
            }
            _ => false,
        };

        if !valid {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Type mismatch for param '{}'", param),
            ));
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