use crate::model::QuotaSnapshot;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub refresh_minutes: u32,
    pub alert_threshold: Option<u8>,
    pub session_alert_threshold: Option<u8>,
    pub pace_alerts: bool,
    pub recovery_alerts: bool,
    pub locale: String,
    pub theme: String,
    pub tray_metric: String,
    pub service_status_checks: bool,
    pub global_hotkey: bool,
    pub flyout_pinned: bool,
    pub onboarding_shown: bool,
    pub last_alert_reset: Option<i64>,
    pub last_session_alert_reset: Option<i64>,
    pub last_weekly_pace_alert_reset: Option<i64>,
    pub last_session_pace_alert_reset: Option<i64>,
    pub last_update_check: Option<i64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            refresh_minutes: 5,
            alert_threshold: None,
            session_alert_threshold: None,
            pace_alerts: false,
            recovery_alerts: false,
            locale: "auto".to_owned(),
            theme: "system".to_owned(),
            tray_metric: "weekly".to_owned(),
            service_status_checks: true,
            global_hotkey: false,
            flyout_pinned: false,
            onboarding_shown: false,
            last_alert_reset: None,
            last_session_alert_reset: None,
            last_weekly_pace_alert_reset: None,
            last_session_pace_alert_reset: None,
            last_update_check: None,
        }
    }
}

impl Settings {
    pub fn normalize(&mut self) {
        if !matches!(self.refresh_minutes, 1 | 5 | 15) {
            self.refresh_minutes = 5;
        }
        if !matches!(self.alert_threshold, None | Some(10 | 20 | 30)) {
            self.alert_threshold = None;
        }
        if !matches!(self.session_alert_threshold, None | Some(10 | 20 | 30)) {
            self.session_alert_threshold = None;
        }
        if !matches!(self.locale.as_str(), "auto" | "en" | "zh-CN") {
            self.locale = "auto".to_owned();
        }
        if !matches!(self.theme.as_str(), "system" | "light" | "dark") {
            self.theme = "system".to_owned();
        }
        if !matches!(self.tray_metric.as_str(), "weekly" | "session" | "lowest") {
            self.tray_metric = "weekly".to_owned();
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppStore {
    directory: PathBuf,
}

impl AppStore {
    pub fn discover() -> Self {
        let base =
            std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let directory = base.join(app_dir_for(env!("CODEX_STATUS_CHANNEL")));
        if let Some(legacy) = legacy_app_dir_for(env!("CODEX_STATUS_CHANNEL")) {
            let _ = migrate_legacy_store(&base.join(legacy), &directory);
        }
        Self { directory }
    }

    #[cfg(test)]
    pub fn at(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn load_settings(&self) -> Settings {
        let mut settings =
            read_json::<Settings>(&self.directory.join("settings.json")).unwrap_or_default();
        settings.normalize();
        settings
    }

    pub fn save_settings(&self, settings: &Settings) -> io::Result<()> {
        write_json_atomic(&self.directory.join("settings.json"), settings)
    }

    pub fn load_snapshot(&self) -> Option<QuotaSnapshot> {
        read_json(&self.directory.join("snapshot.json"))
    }

    pub fn save_snapshot(&self, snapshot: &QuotaSnapshot) -> io::Result<()> {
        write_json_atomic(&self.directory.join("snapshot.json"), snapshot)
    }

    pub fn updates_directory(&self) -> PathBuf {
        self.directory.join("updates")
    }
}

fn app_dir_for(channel: &str) -> &'static str {
    match channel {
        "stable" => "CodexStatus",
        "beta" => "CodexStatusBeta",
        "development" => "CodexStatusDevelopment",
        "portable" => "CodexStatusPortable",
        _ => unreachable!("build.rs validates CODEX_STATUS_CHANNEL"),
    }
}

fn legacy_app_dir_for(channel: &str) -> Option<&'static str> {
    match channel {
        // Portable builds before channel isolation used the stable directory.
        "portable" => Some("CodexStatus"),
        "stable" | "beta" | "development" => None,
        _ => unreachable!("build.rs validates CODEX_STATUS_CHANNEL"),
    }
}

fn migrate_legacy_store(source: &Path, target: &Path) -> io::Result<()> {
    for name in ["settings.json", "snapshot.json"] {
        copy_if_missing(&source.join(name), &target.join(name))?;
    }
    Ok(())
}

fn copy_if_missing(source: &Path, target: &Path) -> io::Result<()> {
    if !source.is_file() || target.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = target.with_extension("migration.tmp");
    fs::copy(source, &temporary)?;
    fs::rename(temporary, target)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    fs::write(&temporary, bytes)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn normalizes_untrusted_settings() {
        let mut settings = Settings {
            refresh_minutes: 2,
            alert_threshold: Some(99),
            session_alert_threshold: Some(99),
            locale: "invalid".to_owned(),
            theme: "sepia".to_owned(),
            tray_metric: "random".to_owned(),
            ..Settings::default()
        };
        settings.normalize();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn round_trips_settings_atomically() {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let directory = std::env::temp_dir().join(format!("codex-status-settings-{suffix}"));
        let store = AppStore::at(directory.clone());
        let settings = Settings {
            refresh_minutes: 15,
            alert_threshold: Some(20),
            session_alert_threshold: Some(10),
            pace_alerts: true,
            recovery_alerts: true,
            tray_metric: "lowest".to_owned(),
            service_status_checks: false,
            global_hotkey: true,
            flyout_pinned: true,
            ..Settings::default()
        };
        store.save_settings(&settings).unwrap();
        assert_eq!(store.load_settings(), settings);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn channel_directories_are_distinct_and_stable_keeps_its_published_path() {
        assert_eq!(app_dir_for("stable"), "CodexStatus");
        assert_eq!(app_dir_for("beta"), "CodexStatusBeta");
        assert_eq!(app_dir_for("development"), "CodexStatusDevelopment");
        assert_eq!(app_dir_for("portable"), "CodexStatusPortable");
        assert_eq!(legacy_app_dir_for("portable"), Some("CodexStatus"));
        assert_eq!(legacy_app_dir_for("stable"), None);
    }

    #[test]
    fn portable_migration_copies_existing_state_without_overwriting_new_state() {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("codex-status-migration-{suffix}"));
        let legacy = root.join("CodexStatus");
        let portable = root.join("CodexStatusPortable");
        let legacy_store = AppStore::at(legacy.clone());
        let portable_store = AppStore::at(portable.clone());
        let legacy_settings = Settings { refresh_minutes: 15, ..Settings::default() };
        legacy_store.save_settings(&legacy_settings).unwrap();

        migrate_legacy_store(&legacy, &portable).unwrap();
        assert_eq!(portable_store.load_settings(), legacy_settings);

        let portable_settings = Settings { refresh_minutes: 1, ..Settings::default() };
        portable_store.save_settings(&portable_settings).unwrap();
        legacy_store.save_settings(&Settings::default()).unwrap();
        migrate_legacy_store(&legacy, &portable).unwrap();
        assert_eq!(portable_store.load_settings(), portable_settings);
        fs::remove_dir_all(root).unwrap();
    }
}
