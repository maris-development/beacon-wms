use axum::http::HeaderMap;
use sha2::Digest;

use crate::misc;
use crate::query_parameters::GetMapRequestParameters;

/// Identity of one tile, for the cache on disk and for the cache of the client.
///
/// The tag covers the request parameters, the config file and every dataset file
/// that the tile draws from. A refresh writes a new dataset file, so its file time
/// moves and the tag changes with it. The build step reads file metadata only, so a
/// client can revalidate without a read of the tile and without a render.
pub struct TileValidator {
    /// Hex digest of all inputs. The tile cache uses it as the file name.
    key: String,
    /// Newest input time, in whole seconds since the epoch.
    modified_seconds: u64,
}

impl TileValidator {
    pub fn build(
        get_map_params: &GetMapRequestParameters,
        layer_filepaths: &[String],
    ) -> Self {
        let config_seconds = misc::config_modified_seconds();
        let mut modified_seconds = config_seconds;

        let mut hasher = sha2::Sha256::new();
        hasher.update(get_map_params.hash().as_bytes());
        hasher.update(config_seconds.to_le_bytes());

        for filepath in layer_filepaths {
            let file_seconds = misc::file_modified_seconds(filepath);

            hasher.update(filepath.as_bytes());
            hasher.update(file_seconds.to_le_bytes());

            modified_seconds = modified_seconds.max(file_seconds);
        }

        TileValidator {
            key: format!("{:x}", hasher.finalize()),
            modified_seconds,
        }
    }

    /// File name of the tile in the cache on disk.
    pub fn cache_key(&self) -> &str {
        &self.key
    }

    /// Value for the `ETag` header.
    ///
    /// The tag is weak. It names the inputs of the tile, not the bytes of one render.
    pub fn etag(&self) -> String {
        format!("W/\"{}\"", self.key)
    }

    /// Value for the `Last-Modified` header.
    pub fn last_modified(&self) -> String {
        misc::format_http_date(self.modified_seconds)
    }

    /// True when the client holds the current tile.
    ///
    /// `If-None-Match` wins over `If-Modified-Since`, as RFC 9110 demands.
    pub fn is_fresh_for(&self, headers: &HeaderMap) -> bool {
        if let Some(value) = headers.get("If-None-Match").and_then(|v| v.to_str().ok()) {
            return self.matches_etag(value);
        }

        let Some(value) = headers
            .get("If-Modified-Since")
            .and_then(|v| v.to_str().ok())
        else {
            return false;
        };

        match misc::parse_http_date(value) {
            Some(client_seconds) => self.modified_seconds <= client_seconds,
            None => false,
        }
    }

    fn matches_etag(&self, header_value: &str) -> bool {
        header_value.split(',').any(|candidate| {
            let candidate = candidate.trim();

            candidate == "*" || strip_weak_prefix(candidate) == strip_weak_prefix(&self.etag())
        })
    }
}

/// Compare tags by their opaque part. A weak tag validates the same tile.
fn strip_weak_prefix(etag: &str) -> &str {
    etag.strip_prefix("W/").unwrap_or(etag)
}
