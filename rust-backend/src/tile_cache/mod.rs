use std::path::PathBuf;

use tokio::fs::File;

use crate::query_parameters::GetMapRequestParameters;

#[derive(Clone, Debug)]
pub struct TileCache {
    tile_cache_directory: String
}

impl TileCache {
    pub fn new(tile_cache_directory: String) -> Self {
        TileCache {
            tile_cache_directory,
        }
    }

    fn hash_prefix(hash: &str) -> &str {
        hash.get(..2).unwrap_or("xx")
    }

    fn tile_path_for_hash(&self, tile_hash: &str, extension: &str) -> PathBuf {
        let mut dir = PathBuf::from(&self.tile_cache_directory);
        dir.push(Self::hash_prefix(tile_hash));
        dir.push(format!("{}.{}", tile_hash, extension));
        dir
    }

    pub async fn is_cached(&self, get_map_params: &GetMapRequestParameters, extension: &str) -> Option<File> {
        let tile_hash = get_map_params.hash();
        let tile_path = self.tile_path_for_hash(&tile_hash, extension);

        File::open(tile_path).await.ok()
    }

    pub async fn cache_tile(
        &self,
        get_map_params: &GetMapRequestParameters,
        tile_data: &[u8],
        extension: &str,
    ) -> std::io::Result<()> {
        let tile_hash = get_map_params.hash();
        let tile_path = self.tile_path_for_hash(&tile_hash, extension);

        if let Some(parent) = tile_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tokio::fs::write(tile_path, tile_data).await
    }

    pub async fn clear_cache(&self) -> std::io::Result<()> {
        let cache_root = PathBuf::from(&self.tile_cache_directory);

        if tokio::fs::metadata(&cache_root).await.is_ok() {
            tokio::fs::remove_dir_all(&cache_root).await?;
        }

        tokio::fs::create_dir_all(&cache_root).await
    }
}