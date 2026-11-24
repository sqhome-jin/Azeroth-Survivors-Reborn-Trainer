use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value};

const DIRECTORY: &str = "Azeroth Survivor Reborn Trainer";
const FILE: &str = "reborn-settings.json";

#[cfg(windows)]
pub fn state_directory() -> io::Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(|base| PathBuf::from(base).join(DIRECTORY))
        .ok_or_else(|| io::Error::other("No Windows user settings directory."))
}

#[cfg(not(windows))]
pub fn state_directory() -> io::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(|base| {
            PathBuf::from(base)
                .join("Library")
                .join("Application Support")
                .join(DIRECTORY)
        })
        .ok_or_else(|| io::Error::other("No user settings directory (HOME is not set)."))
}

#[derive(Default)]
pub struct Settings {
    path: Option<PathBuf>,
    fields: Map<String, Value>,
    pub recovery_notice: Option<String>,
}

impl Settings {
    pub fn load() -> io::Result<Self> {
        let path = state_directory()?.join(FILE);
        let legacy = std::env::current_exe()?.with_file_name(FILE);
        Self::from_paths(path, &legacy)
    }

    fn from_paths(path: PathBuf, legacy: &Path) -> io::Result<Self> {
        let mut imported = false;
        let text = match fs::read(&path) {
            Ok(text) => Some(text),
            Err(err) if err.kind() == io::ErrorKind::NotFound => match fs::read(legacy) {
                Ok(text) => {
                    imported = true;
                    Some(text)
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => None,
                Err(err) => return Err(err),
            },
            Err(err) => return Err(err),
        };
        let fields = text.as_deref().map(serde_json::from_slice).transpose();
        let (fields, recovery_notice) = match fields {
            Ok(fields) => (fields.unwrap_or_default(), None),
            Err(_) => {
                if !imported {
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_err(io::Error::other)?
                        .as_nanos();
                    fs::rename(&path, path.with_extension(format!("invalid-{stamp}.json")))?;
                }
                (
                    Map::new(),
                    Some(
                        "Settings were invalid. Defaults loaded and the original was preserved."
                            .into(),
                    ),
                )
            }
        };
        Ok(Self {
            path: Some(path),
            fields,
            recovery_notice,
        })
    }

    pub fn game_path(&self) -> Option<&Path> {
        self.fields
            .get("game_exe")
            .and_then(Value::as_str)
            .map(Path::new)
    }

    pub fn stat_preferences(&self) -> Option<&Value> {
        self.fields.get("stats")
    }

    pub fn set_game_path(&mut self, path: &Path) -> io::Result<()> {
        let path = path
            .to_str()
            .ok_or_else(|| io::Error::other("Game path is not Unicode."))?;
        self.save("game_exe", Value::String(path.into()))
    }

    pub fn set_stat_preferences(&mut self, preferences: Value) -> io::Result<()> {
        self.save("stats", preferences)
    }

    fn save(&mut self, key: &str, value: Value) -> io::Result<()> {
        let Some(file) = &self.path else {
            return Err(io::Error::other("Settings could not be loaded."));
        };
        let mut fields = self.fields.clone();
        fields.insert(key.into(), value);
        fs::create_dir_all(file.parent().expect("settings parent"))?;
        let temp = file.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temp, serde_json::to_vec_pretty(&fields)?)?;
        let result = fs::rename(&temp, file);
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result?;
        self.fields = fields;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_settings_are_preserved_and_can_be_replaced() {
        let folder = std::env::temp_dir().join(format!(
            "reborn-settings-recovery-test-{}",
            std::process::id()
        ));
        let current = folder.join(FILE);
        let legacy = folder.join("legacy.json");
        fs::create_dir_all(&folder).unwrap();
        fs::write(&current, b"{broken").unwrap();
        let mut settings = Settings::from_paths(current.clone(), &legacy).unwrap();
        assert!(settings.recovery_notice.is_some());
        assert!(!current.exists());
        let backup = fs::read_dir(&folder).unwrap().next().unwrap().unwrap();
        assert_eq!(fs::read(backup.path()).unwrap(), b"{broken");
        settings
            .set_stat_preferences(serde_json::json!({}))
            .unwrap();
        assert!(
            Settings::from_paths(current.clone(), &legacy)
                .unwrap()
                .stat_preferences()
                .is_some()
        );
        fs::remove_file(backup.path()).unwrap();
        fs::remove_file(current).unwrap();
        fs::remove_dir(folder).unwrap();
    }

    #[test]
    fn imports_legacy_settings_and_preserves_unrelated_preferences() {
        let folder =
            std::env::temp_dir().join(format!("reborn-settings-test-{}", std::process::id()));
        let legacy = folder.join(FILE);
        let current = folder.join("user").join(FILE);
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            &legacy,
            br#"{"game_exe":"C:/Game/AzerothSurvivorsGame.exe","stats":{"enabled":["damage"]}}"#,
        )
        .unwrap();
        let mut settings = Settings::from_paths(current.clone(), &legacy).unwrap();
        assert_eq!(
            settings.game_path(),
            Some(Path::new("C:/Game/AzerothSurvivorsGame.exe"))
        );
        settings
            .set_game_path(Path::new("D:/Game/AzerothSurvivorsGame.exe"))
            .unwrap();
        let written: Value = serde_json::from_slice(&fs::read(&current).unwrap()).unwrap();
        assert_eq!(written["stats"]["enabled"][0], "damage");
        let retained = Settings::from_paths(current.clone(), &legacy).unwrap();
        assert_eq!(
            retained.game_path(),
            Some(Path::new("D:/Game/AzerothSurvivorsGame.exe"))
        );
        fs::remove_file(current).unwrap();
        fs::remove_file(legacy).unwrap();
    }
}
