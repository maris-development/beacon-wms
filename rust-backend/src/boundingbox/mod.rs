use core::fmt;
use crate::misc::{self};

#[derive(std::fmt::Debug, Clone)]
pub struct BoundingBox {
    /// Minimum longitude (west)
    min_x: f64,
    /// Minimum latitude (south)
    min_y: f64,
    /// Maximum longitude (east)
    max_x: f64,
    /// Maximum latitude (north)
    max_y: f64,
    /// Which projection is used
    projection: String,

    max_bounds: MaxBounds,

}


impl BoundingBox {
    pub fn world() -> Self {
        BoundingBox::new(-180.0, -90.0, 180.0, 90.0, "EPSG:4326")
    }

    pub fn from_string(bbox: &str, crs: &str, wms_version: &str) -> Result<Self, String> {
        let parts: Vec<&str> = bbox.split(",").collect();

        if parts.len() != 4 {
            return Err(String::from("Invalid number of parts in BoundingBox string"));
        }

        // log::info!("Creating bounding box from string: {} with crs: {} and wms version: {}", bbox, crs, wms_version);

        let min_x: f64;
        let min_y: f64;
        let max_x: f64;
        let max_y: f64;

        if crs.to_uppercase() == "EPSG:4326" {
                match wms_version {
                    "1.3.0" => {
                        min_y = parts[0].parse::<f64>().map_err(|e| e.to_string())?;
                        min_x = parts[1].parse::<f64>().map_err(|e| e.to_string())?;
                        max_y = parts[2].parse::<f64>().map_err(|e| e.to_string())?;
                        max_x = parts[3].parse::<f64>().map_err(|e| e.to_string())?;
                    }

                    "1.1.1" => {
                        min_x = parts[0].parse::<f64>().map_err(|e| e.to_string())?;
                        min_y = parts[1].parse::<f64>().map_err(|e| e.to_string())?;
                        max_x = parts[2].parse::<f64>().map_err(|e| e.to_string())?;
                        max_y = parts[3].parse::<f64>().map_err(|e| e.to_string())?;
                    }

                    _ => return Err(String::from("Invalid WMS version, use 1.3.0 or 1.1.1"))
                }
        } else {
            // For other projections, always use minx,miny,maxx,maxy
            min_x = parts[0].parse::<f64>().map_err(|e| e.to_string())?;
            min_y = parts[1].parse::<f64>().map_err(|e| e.to_string())?;
            max_x = parts[2].parse::<f64>().map_err(|e| e.to_string())?;
            max_y = parts[3].parse::<f64>().map_err(|e| e.to_string())?;
        }
    

        // log::info!("Parsed bbox: min_x: {}, min_y: {}, max_x: {}, max_y: {},  ", min_x, min_y, max_x, max_y);

        Ok(BoundingBox::new(min_x, min_y, max_x, max_y, crs))
    }


    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64, projection: &str) -> Self {

        let projection = projection.to_uppercase();

        let max_bounds = MaxBounds::new(-180.0, -90.0, 180.0, 90.0, "EPSG:4326").reproject(projection.as_str());

        if max_bounds.is_err() {
            panic!("Could not reproject max bounds to projection: {}", projection);
        }

        let max_bounds = max_bounds.unwrap();
        
        BoundingBox {
            min_x,
            min_y,
            max_x,
            max_y,
            projection: String::from(projection),
            max_bounds,
        }
    }

    pub fn get_min_x(&self) -> f64 {
        self.min_x
    }

    pub fn get_min_y(&self) -> f64 {
        self.min_y
    }

    pub fn get_max_x(&self) -> f64 {
        self.max_x
    }

    pub fn get_max_y(&self) -> f64 {
        self.max_y
    }

    pub fn is_correct(&self) -> bool {
        self.min_x <= self.max_x && self.min_y <= self.max_y
    }

    pub fn get_center_x(&self) -> f64 {
        (self.max_x + self.min_x) / 2.0
    }

    pub fn get_width(&self) -> f64 {
        self.max_x - self.min_x
    }

    pub fn get_width_degrees(&self) -> f64 {
        let max_bounds = self.get_max_bounds();
        let width = self.get_width();
        let max_width = max_bounds.get_width();

        width / max_width * 360.0
    }

    pub fn get_height(&self) -> f64 {
        self.max_y - self.min_y
    }

    pub fn get_max_bounds(&self) -> &MaxBounds {
        &self.max_bounds
    }

    /// Scales the boundingbox to the given factor.
    /// Primarily used for retrieving features outside of the bounding box, so we don't miss any.
    pub fn scale(&self, factor: f64) -> BoundingBox {
        let width = self.get_width();
        let height = self.get_height();

        let new_width = width * factor;
        let new_height = height * factor;

        let new_min_x = self.min_x - (new_width - width) / 2.0;
        let new_min_y = self.min_y - (new_height - height) / 2.0;
        let new_max_x = self.max_x + (new_width - width) / 2.0;
        let new_max_y = self.max_y + (new_height - height) / 2.0;

        BoundingBox::new(new_min_x, new_min_y, new_max_x, new_max_y, &self.projection)
    }

    /// Check if coordinates fall in the current boundingbox, with a margin.
    /// If margin is None, no margin is used.
    /// Coordinates are in the projection of the boundingbox in XY order.
    pub fn in_bbox(&self, coordinates: (f64, f64), margin: Option<f64>) -> bool {
        let margin = match margin {
            Some(margin) => margin,
            None => 0.0,
        };

        let (x, y) = coordinates;
        let in_y_boundary = y >= (self.min_y - margin) && y <= (self.max_y + margin);
        let in_x_boundary = x >= (self.min_x - margin) && x <= (self.max_x + margin);

        let in_normal_boundary = in_x_boundary && in_y_boundary;

        if margin <= 0.0 {
            return in_normal_boundary;
        }

        if !in_normal_boundary && in_y_boundary && self.max_bounds.on_left_boundary(self.min_x) {
            
            let over_left_boundary_but_in_margin =
                x >= self.max_bounds.get_max_x() - margin && x <= self.max_bounds.get_max_x();

            return over_left_boundary_but_in_margin;

        } else if !in_normal_boundary
            && in_y_boundary
            && self.max_bounds.on_right_boundary(self.get_max_x())
        {
            let over_right_boundary_in_margin =
                x >= self.max_bounds.get_min_x() && x <= self.max_bounds.get_min_x() + margin;

            return over_right_boundary_in_margin;
        }

        return in_normal_boundary;
    }

    pub fn get_projection_code(&self) -> &str {
        self.projection.as_str()
    }

    pub fn reproject(&self, target_projection_code: &str) -> Result<Self, String> {
        let target_projection_code =  target_projection_code.to_uppercase();
        let target_projection_code = target_projection_code.as_str();

        if self.get_projection_code() == target_projection_code {
            return Ok(self.clone());
        }

        let mut min_xy = (self.min_x, self.min_y);
        let mut max_xy = (self.max_x, self.max_y);
        
        if let Err(err) = misc::transform_coordinates(self.get_projection_code(), target_projection_code, &mut min_xy) {
            return Err(err);
        }

        if let Err(err) = misc::transform_coordinates(self.get_projection_code(), target_projection_code, &mut max_xy) {
            return Err(err);
        }

        Ok(Self::new(min_xy.0, min_xy.1, max_xy.0, max_xy.1, target_projection_code))
    }
}

// To use the `{}` marker, the trait `fmt::Display` must be implemented
// manually for the type.
impl fmt::Display for BoundingBox {
    // This trait requires `fmt` with this exact signature.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "BoundingBox({}, {}, {}, {}, {})",
            self.min_x, self.min_y, self.max_x, self.max_y, self.projection
        )
    }
}



#[derive(std::fmt::Debug, Clone)]
pub struct MaxBounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    projection: String,
}

impl MaxBounds {
    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64, projection: &str) -> Self {
        MaxBounds {
            min_x,
            min_y,
            max_x,
            max_y,
            projection: String::from(projection),
        }
    }
    pub fn get_min_x(&self) -> f64 {
        self.min_x
    }

    pub fn get_min_y(&self) -> f64 {
        self.min_y
    }

    pub fn get_max_x(&self) -> f64 {
        self.max_x
    }

    pub fn get_max_y(&self) -> f64 {
        self.max_y
    }

    pub fn on_right_boundary(&self, x: f64) -> bool {
        x >= self.max_x
    }

    pub fn on_left_boundary(&self, x: f64) -> bool {
        x <= self.min_x
    }

    pub fn get_width(&self) -> f64 {
        self.max_x - self.min_x
    }

    pub fn get_projection_code(&self) -> &str {
        self.projection.as_str()
    }

    pub fn reproject(&self, target_projection_code: &str) -> Result<Self, String> {
        let target_projection_code =  target_projection_code.to_uppercase();
        let target_projection_code = target_projection_code.as_str();

        if self.get_projection_code() == target_projection_code {
            return Ok(Self::new(self.min_x, self.min_y, self.max_x, self.max_y, target_projection_code))
        }

        let mut min_xy = (self.min_x, self.min_y);
        let mut max_xy = (self.max_x, self.max_y);
        
        if let Err(err) = misc::transform_coordinates(self.get_projection_code(), target_projection_code, &mut min_xy) {
            log::error!("Error transforming coordinates: {} : {:?} {} -> {}", err, (self.min_x, self.min_y), self.get_projection_code(), target_projection_code);
            return Err(err);
        }

        if let Err(err) = misc::transform_coordinates(self.get_projection_code(), target_projection_code, &mut max_xy) {
            log::error!("Error transforming coordinates: {} : {:?} {} -> {}", err, (self.max_x, self.max_y), self.get_projection_code(), target_projection_code);
            return Err(err);
        }

        Ok(Self::new(min_xy.0, min_xy.1, max_xy.0, max_xy.1, target_projection_code))
    }
}

// To use the `{}` marker, the trait `fmt::Display` must be implemented
// manually for the type.
impl fmt::Display for MaxBounds {
    // This trait requires `fmt` with this exact signature.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "MaxBounds({}, {}, {}, {}, {})",
            self.min_x, self.min_y, self.max_x, self.max_y, self.projection
        )
    }
}
