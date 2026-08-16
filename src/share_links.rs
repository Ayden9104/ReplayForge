//! Persist cloud share URLs per clip so Create/Copy does not re-upload.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Matches ~7-day R2 lifecycle with a small safety margin (~6.5 days).
const LIVE_SECS: u64 = 561_600;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareLinkEntry {
    pub url: String,
    /// Unix seconds when the link was created.
    pub created: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ShareLinksFile {
    #[serde(default)]
    links: HashMap<String, ShareLinkEntry>,
}

#[derive(Debug, Default)]
pub struct ShareLinkStore {
    links: HashMap<String, ShareLinkEntry>,
}

impl ShareLinkStore {
    pub fn load() -> Self {
        let Some(path) = Self::store_path() else {
            return Self::default();
        };
        match fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str::<ShareLinksFile>(&contents) {
                Ok(mut file) => {
                    let now = now_secs();
                    file.links
                        .retain(|_, entry| now.saturating_sub(entry.created) < LIVE_SECS);
                    Self { links: file.links }
                }
                Err(error) => {
                    eprintln!("Failed to parse share_links.toml: {error}");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    pub fn store_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("com", "ReplayForge", "ReplayForge")
            .map(|dirs| dirs.config_dir().join("share_links.toml"))
    }

    /// Live share URL if still within the retention window.
    pub fn get_live(&self, path: &Path) -> Option<&str> {
        let entry = self.links.get(&path_key(path))?;
        if now_secs().saturating_sub(entry.created) < LIVE_SECS {
            Some(entry.url.as_str())
        } else {
            None
        }
    }

    pub fn has_live(&self, path: &Path) -> bool {
        self.get_live(path).is_some()
    }

    /// Return live URL, or remove a stale entry and return None.
    pub fn take_live_or_clear_stale(&mut self, path: &Path) -> Option<String> {
        let key = path_key(path);
        match self.links.get(&key) {
            Some(entry) if now_secs().saturating_sub(entry.created) < LIVE_SECS => {
                Some(entry.url.clone())
            }
            Some(_) => {
                self.links.remove(&key);
                let _ = self.save();
                None
            }
            None => None,
        }
    }

    pub fn put(&mut self, path: &Path, url: String) {
        let key = path_key(path);
        self.links.insert(
            key,
            ShareLinkEntry {
                url,
                created: now_secs(),
            },
        );
        let _ = self.save();
    }

    pub fn remove(&mut self, path: &Path) {
        let key = path_key(path);
        if self.links.remove(&key).is_some() {
            let _ = self.save();
        }
    }

    /// Move a stored link when a clip file is renamed.
    pub fn rename_path(&mut self, old: &Path, new: &Path) {
        let old_key = path_key(old);
        let Some(entry) = self.links.remove(&old_key) else {
            // Fallback: raw display path if canonicalize differed before rename.
            let raw = old.to_string_lossy().into_owned();
            let Some(entry) = self.links.remove(&raw) else {
                return;
            };
            self.links.insert(path_key(new), entry);
            let _ = self.save();
            return;
        };
        self.links.insert(path_key(new), entry);
        let _ = self.save();
    }

    fn save(&self) -> Result<(), String> {
        let path =
            Self::store_path().ok_or_else(|| "Could not resolve share_links path".to_string())?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create config dir: {e}"))?;
        }
        let file = ShareLinksFile {
            links: self.links.clone(),
        };
        let contents = toml::to_string_pretty(&file)
            .map_err(|e| format!("Failed to serialize share_links: {e}"))?;
        fs::write(&path, contents).map_err(|e| format!("Failed to write share_links: {e}"))?;
        Ok(())
    }
}

fn path_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
