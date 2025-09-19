use thiserror::Error;

use crate::boundingbox::BoundingBox;

#[derive(Error, Debug)]
pub enum MapError {
    #[error("{0}")]
    BbfError(String),

    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Projection create error: {0}")]
    ProjError(#[from] proj4rs::errors::Error),

    #[error("Incorrect BoundingBox: {0}")]
    BoundingBoxError(BoundingBox),

    #[error("ColorMap Error: {0}")]
    ColorMapError(String),

    #[error("Drawing Error: {0}")]
    Error(String),

    #[error("Unknown map draw error")]
    Unknown,
}