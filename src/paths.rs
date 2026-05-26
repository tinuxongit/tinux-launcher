use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::path::{Path, PathBuf};

pub struct Paths {
    pub root: PathBuf,
    pub versions: PathBuf,
    pub libraries: PathBuf,
    pub assets: PathBuf,
    pub assets_indexes: PathBuf,
    pub assets_objects: PathBuf,
    pub natives: PathBuf,
    pub instances: PathBuf,
    pub vanilla_minecraft: PathBuf,
    pub logs: PathBuf,
    pub cache: PathBuf,
}

impl Paths {
    pub fn resolve() -> Result<Self> {
        let dirs = ProjectDirs::from("dev", "tinux", "TinuxLauncher")
            .context("could not resolve project directories")?;
        let root = dirs.data_dir().to_path_buf();
        let assets = root.join("assets");
        let s = Self {
            versions: root.join("versions"),
            libraries: root.join("libraries"),
            assets_indexes: assets.join("indexes"),
            assets_objects: assets.join("objects"),
            assets,
            natives: root.join("natives"),
            instances: root.join("instances"),
            vanilla_minecraft: root.join(".minecraft"),
            logs: root.join("logs"),
            cache: dirs.cache_dir().to_path_buf(),
            root,
        };
        s.ensure_dirs()?;
        Ok(s)
    }

    fn ensure_dirs(&self) -> Result<()> {
        for p in [
            &self.root,
            &self.versions,
            &self.libraries,
            &self.assets,
            &self.assets_indexes,
            &self.assets_objects,
            &self.natives,
            &self.instances,
            &self.vanilla_minecraft,
            &self.logs,
            &self.cache,
        ] {
            std::fs::create_dir_all(p)
                .with_context(|| format!("creating {}", p.display()))?;
        }
        Ok(())
    }

    pub fn version_dir(&self, id: &str) -> PathBuf {
        self.versions.join(id)
    }

    pub fn version_json(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.json"))
    }

    pub fn version_jar(&self, id: &str) -> PathBuf {
        self.version_dir(id).join(format!("{id}.jar"))
    }

    pub fn natives_dir(&self, id: &str) -> PathBuf {
        self.natives.join(id)
    }

    pub fn asset_object(&self, hash: &str) -> PathBuf {
        self.assets_objects.join(&hash[..2]).join(hash)
    }

    pub fn library_path(&self, sub: &str) -> PathBuf {
        self.libraries.join(sub)
    }
}

pub fn ensure_parent(p: &Path) -> Result<()> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating parent {}", parent.display()))?;
    }
    Ok(())
}
