use serde::{Deserialize, Serialize};
use serde_json::{Value};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Feature {
    #[serde(rename = "type")]
    _type: String,
    geometry: Value,
    properties: Option<serde_json::Value>,
}

impl Feature {
    fn new(geometry: Value, properties: Option<serde_json::Value>) -> Feature {
        Feature {
            _type: String::from("Feature"),
            geometry,
            properties,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetFeatureInfoCollection {
    #[serde(rename = "type")]
    _type: String,
    features: Vec<Feature>,
    properties: Option<serde_json::Value>,
}

impl GetFeatureInfoCollection {
    pub fn new(
        features: Vec<Feature>,
        properties: Option<serde_json::Value>,
    ) -> GetFeatureInfoCollection {
        GetFeatureInfoCollection {
            _type: String::from("GetFeatureInfoCollection"),
            features,
            properties,
        }
    }

    pub fn to_json_string(&self) -> String {
        serde_json::to_string(&self).unwrap()
    }

    pub fn to_html(&self) -> String {
        let mut html = String::new();

        html += "<body>";

        if !self.features.is_empty() {
            html += "<b>Features found:</b>";

            let first_feature = &self.features[0];
            let cols: Vec<String> = match &first_feature.properties {
                Some(properties) => properties
                    .as_object()
                    .map_or(Vec::new(), |map| map.keys().cloned().collect()),
                None => Vec::new(),
            };

            html += "<table><thead><tr>";
            for col in &cols {
                html += format!("<th>{}</th>", col).as_str();
            }
            html += "</tr></thead><tbody>";

            for feature in &self.features {
                html += "<tr>";
                if let Some(properties) = &feature.properties {
                    for col in &cols {
                        if let Some(value) = properties.get(col) {
                             if let Some(s) = value.as_str() {
                                // it's a JSON string, print without quotes
                                html += &format!("<td>{}</td>", s);
                            } else {
                                // numbers, bools, objects, arrays: just print normally
                                html += &format!("<td>{}</td>", value);
                            }
                            // let value_str = match value {
                            //     Value::Number(num) => {
                            //         if let Some(f) = num.as_f64() {
                            //             format!("{:.2}", f)
                            //         } else {
                            //             num.to_string()
                            //         }
                            //     }
                            //     _ => value.to_string(),
                            // };
                        }
                    }
                }
                html += "</tr>";
            }

            html += "</tbody></table>";
        } else {
            html += "<b>No features found</b>";
        }

        html += "</body>";

        html
    }

    pub fn to_xml(&self) -> String {
        let mut xml = String::new();

        xml += r#"<?xml version="1.0" encoding="UTF-8"?>"#;
        xml += r#"<FeatureCollection xmlns:gml="http://www.opengis.net/gml">"#;

        for (i, feature) in self.features.iter().enumerate() {
            xml += "<gml:featureMember>";

            xml += &format!(r#"<Feature gml:id="feature.{}">"#, i + 1);

            // Geometry (Point only in your use case)
            if let Some(geom_obj) = feature.geometry.as_object() {
                if let (Some(geom_type), Some(coords)) =
                    (geom_obj.get("type"), geom_obj.get("coordinates"))
                {
                    if geom_type == "Point" {
                        if let Some(coords_arr) = coords.as_array() {
                            if coords_arr.len() == 2 {
                                let lng = coords_arr[0].as_f64().unwrap_or(0.0);
                                let lat = coords_arr[1].as_f64().unwrap_or(0.0);

                                xml += "<gml:Point srsName=\"EPSG:4326\">";
                                xml += &format!("<gml:pos>{} {}</gml:pos>", lng, lat);
                                xml += "</gml:Point>";
                            }
                        }
                    }
                }
            }

            // Properties
            if let Some(props) = &feature.properties {
                if let Some(map) = props.as_object() {
                    for (key, value) in map {
                        xml += &format!(
                            "<{k}>{v}</{k}>",
                            k = key,
                            v = match value {
                                serde_json::Value::String(s) => s.clone(),
                                serde_json::Value::Number(n) => n.to_string(),
                                serde_json::Value::Bool(b) => b.to_string(),
                                _ => value.to_string(),
                            }
                        );
                    }
                }
            }

            xml += "</Feature>";
            xml += "</gml:featureMember>";
        }

        xml += "</FeatureCollection>";

        xml
    }

    pub fn create_point_feature(
        lng: f64,
        lat: f64,
        properties: Option<serde_json::Map<String, Value>>,
    ) -> Feature {



        let mut _geometry: serde_json::map::Map<String, serde_json::Value> =
            serde_json::map::Map::new();

        _geometry.insert(String::from("type"), serde_json::Value::from("Point"));
        _geometry.insert(
            String::from("coordinates"),
            serde_json::Value::from(vec![lng, lat]),
        );

        Feature::new(
            serde_json::Value::from(_geometry),
            Some(serde_json::Value::from(properties)),
        )
    }
}
