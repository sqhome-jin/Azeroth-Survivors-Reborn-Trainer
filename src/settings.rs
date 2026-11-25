use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const DIRECTORY: &str = "Azeroth Survivor Reborn Trainer";
const FILE: &str = "reborn-settings.json";

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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub game_path: Option<String>,
    pub language: String,
    pub appearance: String,
    pub enabled_stats: Vec<String>,
    pub targets: Map<String, Value>,
    pub recovery_notice: Option<String>,
}

#[derive(Default)]
pub struct Settings {
    path: Option<PathBuf>,
    fields: Map<String, Value>,
    pub recovery_notice: Option<String>,
}

#[allow(dead_code)]
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

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
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

    pub fn language(&self) -> Option<&str> {
        self.fields.get("language").and_then(Value::as_str)
    }

    pub fn appearance(&self) -> Option<&str> {
        self.fields.get("appearance").and_then(Value::as_str)
    }

    pub fn to_dto(&self) -> SettingsDto {
        let prefs = self.stat_preferences();
        let enabled_stats = prefs
            .and_then(|data| data.get("enabled"))
            .and_then(Value::as_array)
            .map(|keys| {
                keys.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_else(|| {
                ["damage", "cooldown", "area", "luck"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            });
        let targets = prefs
            .and_then(|data| data.get("targets"))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        SettingsDto {
            game_path: self.game_path().map(|path| path.display().to_string()),
            language: self.language().unwrap_or("en").to_owned(),
            appearance: self.appearance().unwrap_or("system").to_owned(),
            enabled_stats,
            targets,
            recovery_notice: self.recovery_notice.clone(),
        }
    }

    pub fn apply_dto(&mut self, dto: &SettingsDto) -> io::Result<()> {
        let mut stats = Map::new();
        stats.insert(
            "enabled".into(),
            Value::Array(
                dto.enabled_stats
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        );
        stats.insert("targets".into(), Value::Object(dto.targets.clone()));
        let mut fields = self.fields.clone();
        match &dto.game_path {
            Some(path) => {
                fields.insert("game_exe".into(), Value::String(path.clone()));
            }
            None => {
                fields.remove("game_exe");
            }
        }
        fields.insert("language".into(), Value::String(dto.language.clone()));
        fields.insert("appearance".into(), Value::String(dto.appearance.clone()));
        fields.insert("stats".into(), Value::Object(stats));
        self.write_fields(fields)
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

    pub fn set_language(&mut self, language: &str) -> io::Result<()> {
        self.save("language", Value::String(language.into()))
    }

    pub fn set_appearance(&mut self, appearance: &str) -> io::Result<()> {
        self.save("appearance", Value::String(appearance.into()))
    }

    fn save(&mut self, key: &str, value: Value) -> io::Result<()> {
        let mut fields = self.fields.clone();
        fields.insert(key.into(), value);
        self.write_fields(fields)
    }

    fn write_fields(&mut self, fields: Map<String, Value>) -> io::Result<()> {
        let Some(file) = &self.path else {
            return Err(io::Error::other("Settings could not be loaded."));
        };
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
    fn dto_round_trip_preserves_unrelated_keys() {
        let folder =
            std::env::temp_dir().join(format!("reborn-settings-dto-test-{}", std::process::id()));
        let current = folder.join(FILE);
        let legacy = folder.join("legacy.json");
        fs::create_dir_all(&folder).unwrap();
        fs::write(
            &current,
            br#"{"game_exe":"/Game/AzerothSurvivorsGame","stats":{"enabled":["damage"],"targets":{"damage":700}},"extra":1}"#,
        )
        .unwrap();
        let mut settings = Settings::from_paths(current.clone(), &legacy).unwrap();
        let mut dto = settings.to_dto();
        assert_eq!(dto.enabled_stats, vec!["damage".to_owned()]);
        dto.language = "zh-Hans".into();
        dto.appearance = "dark".into();
        dto.enabled_stats = vec!["luck".into()];
        settings.apply_dto(&dto).unwrap();
        let written: Value = serde_json::from_slice(&fs::read(&current).unwrap()).unwrap();
        assert_eq!(written["language"], "zh-Hans");
        assert_eq!(written["appearance"], "dark");
        assert_eq!(written["stats"]["enabled"][0], "luck");
        assert_eq!(written["extra"], 1);
        fs::remove_file(current).unwrap();
        fs::remove_dir(folder).unwrap();
    }
}
