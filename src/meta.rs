use crate::app::ContentKind;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const META_FILENAME: &str = ".tinux-meta.json";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct InstanceMeta {
    #[serde(default)]
    pub mods: HashMap<String, String>,
    #[serde(default)]
    pub shaders: HashMap<String, String>,
    #[serde(default)]
    pub resourcepacks: HashMap<String, String>,
}

impl InstanceMeta {
    pub fn load(instance_dir: &Path) -> Self {
        let path = instance_dir.join(META_FILENAME);
        std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, instance_dir: &Path) {
        let _ = std::fs::create_dir_all(instance_dir);
        let path = instance_dir.join(META_FILENAME);
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(path, bytes);
        }
    }

    pub fn map_mut(&mut self, kind: ContentKind) -> &mut HashMap<String, String> {
        match kind {
            ContentKind::Mods => &mut self.mods,
            ContentKind::Shaders => &mut self.shaders,
            ContentKind::ResourcePacks => &mut self.resourcepacks,
        }
    }

    pub fn map(&self, kind: ContentKind) -> &HashMap<String, String> {
        match kind {
            ContentKind::Mods => &self.mods,
            ContentKind::Shaders => &self.shaders,
            ContentKind::ResourcePacks => &self.resourcepacks,
        }
    }

    pub fn record(&mut self, kind: ContentKind, project_id: String, filename: String) {
        self.map_mut(kind).insert(project_id, filename);
    }

    pub fn remove_by_filename(&mut self, kind: ContentKind, filename: &str) {
        let map = self.map_mut(kind);
        let to_remove: Option<String> = map
            .iter()
            .find(|(_, v)| v.as_str() == filename)
            .map(|(k, _)| k.clone());
        if let Some(k) = to_remove {
            map.remove(&k);
        }
    }

    pub fn is_installed(&self, kind: ContentKind, project_id: &str) -> bool {
        self.map(kind).contains_key(project_id)
    }
}
