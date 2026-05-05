//! Filesystem JSON cache so reruns are free.
//!
//! Layout under `cache_dir`:
//!   list.json           — designer credit list
//!   items/{id}.json     — per-game geekitems response
//!   dynamic/{id}.json   — per-game dynamicinfo response
use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::path::{Path, PathBuf};

pub struct Cache {
    root: PathBuf,
}

impl Cache {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join("items"))?;
        std::fs::create_dir_all(root.join("dynamic"))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn list_path(&self) -> PathBuf {
        self.root.join("list.json")
    }
    pub fn item_path(&self, id: &str) -> PathBuf {
        self.root.join("items").join(format!("{id}.json"))
    }
    pub fn dynamic_path(&self, id: &str) -> PathBuf {
        self.root.join("dynamic").join(format!("{id}.json"))
    }

    pub fn load<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?;
        if raw.trim().is_empty() {
            return Ok(None);
        }
        let v = serde_json::from_str::<T>(&raw)
            .with_context(|| format!("parse {}", path.display()))?;
        Ok(Some(v))
    }

    pub fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}
