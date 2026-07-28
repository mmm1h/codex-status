use std::io;
use std::path::{Path, PathBuf};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn is_enabled() -> bool {
    std::env::current_exe().is_ok_and(|executable| is_enabled_for(&executable))
}

pub fn is_enabled_for(executable: &Path) -> bool {
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = current_user.open_subkey_with_flags(RUN_KEY, KEY_READ) else {
        return false;
    };
    value_matches(&key, value_name(), executable)
        || legacy_value_name().is_some_and(|name| value_matches(&key, name, executable))
}

pub fn enable(executable: &Path) -> io::Result<()> {
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let access = if legacy_value_name().is_some() { KEY_READ | KEY_WRITE } else { KEY_WRITE };
    let (key, _) = current_user.create_subkey_with_flags(RUN_KEY, access)?;
    let command = startup_command(executable);
    key.set_value(value_name(), &command)?;
    delete_replaced_legacy_value(&key, executable)
}

pub fn disable() -> io::Result<()> {
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let access = if legacy_value_name().is_some() { KEY_READ | KEY_WRITE } else { KEY_WRITE };
    let key = current_user.open_subkey_with_flags(RUN_KEY, access)?;
    delete_value_if_present(&key, value_name())?;
    if let Ok(executable) = std::env::current_exe() {
        delete_replaced_legacy_value(&key, &executable)?;
    }
    Ok(())
}

pub fn migrate_legacy(executable: &Path) -> io::Result<()> {
    let Some(legacy_name) = legacy_value_name() else {
        return Ok(());
    };
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let key = match current_user.open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE) {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let Ok(value) = key.get_value::<String, _>(legacy_name) else {
        return Ok(());
    };
    if !command_matches(&value, executable) {
        return Ok(());
    }
    key.set_value(value_name(), &value)?;
    delete_value_if_present(&key, legacy_name)
}

fn value_name() -> &'static str {
    value_name_for(env!("CODEX_STATUS_CHANNEL"))
}

fn value_name_for(channel: &str) -> &'static str {
    match channel {
        "stable" => "CodexStatus",
        "beta" => "CodexStatusBeta",
        "development" => "CodexStatusDevelopment",
        "portable" => "CodexStatusPortable",
        _ => unreachable!("build.rs validates CODEX_STATUS_CHANNEL"),
    }
}

fn legacy_value_name() -> Option<&'static str> {
    legacy_value_name_for(env!("CODEX_STATUS_CHANNEL"))
}

fn legacy_value_name_for(channel: &str) -> Option<&'static str> {
    match channel {
        // The pre-channel portable build used the stable value name.
        "portable" => Some("CodexStatus"),
        "stable" | "beta" | "development" => None,
        _ => unreachable!("build.rs validates CODEX_STATUS_CHANNEL"),
    }
}

fn value_matches(key: &RegKey, name: &str, executable: &Path) -> bool {
    key.get_value::<String, _>(name).ok().is_some_and(|value| command_matches(&value, executable))
}

fn delete_replaced_legacy_value(key: &RegKey, executable: &Path) -> io::Result<()> {
    let Some(name) = legacy_value_name() else {
        return Ok(());
    };
    let Ok(value) = key.get_value::<String, _>(name) else {
        return Ok(());
    };
    if !legacy_value_is_replaced(&value, executable) {
        return Ok(());
    }
    delete_value_if_present(key, name)
}

fn legacy_value_is_replaced(value: &str, executable: &Path) -> bool {
    command_matches(value, executable)
        || startup_executable(value).is_some_and(|path| !path.is_file())
}

fn delete_value_if_present(key: &RegKey, name: &str) -> io::Result<()> {
    match key.delete_value(name) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn startup_command(executable: &Path) -> String {
    format!("\"{}\" --background", executable.display())
}

fn startup_executable(value: &str) -> Option<PathBuf> {
    let path = value.strip_prefix('"')?.strip_suffix("\" --background")?;
    if path.is_empty() || path.contains('"') {
        return None;
    }
    Some(PathBuf::from(path))
}

fn command_matches(value: &str, executable: &Path) -> bool {
    value.eq_ignore_ascii_case(&startup_command(executable))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_command_quotes_paths_and_adds_background_mode() {
        assert_eq!(
            startup_command(Path::new(r"C:\Program Files\CodexStatus\CodexStatus.exe")),
            r#""C:\Program Files\CodexStatus\CodexStatus.exe" --background"#
        );
    }

    #[test]
    fn stale_or_different_commands_are_not_reported_as_enabled() {
        let executable = Path::new(r"C:\Apps\CodexStatus.exe");
        assert!(command_matches(r#""C:\Apps\CodexStatus.exe" --background"#, executable));
        assert!(command_matches(r#""c:\apps\codexstatus.exe" --background"#, executable));
        assert!(!command_matches(r#""C:\missing\CodexStatus.exe" --background"#, executable));
        assert!(!command_matches(r#""C:\Apps\CodexStatus.exe""#, executable));
    }

    #[test]
    fn legacy_cleanup_only_removes_the_current_or_a_missing_executable() {
        let suffix =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("codex-status-startup-{suffix}"));
        std::fs::create_dir_all(&root).unwrap();
        let current = root.join("current.exe");
        let other = root.join("other.exe");
        std::fs::write(&current, b"current").unwrap();
        std::fs::write(&other, b"other").unwrap();

        assert!(legacy_value_is_replaced(&startup_command(&current), &current));
        assert!(!legacy_value_is_replaced(&startup_command(&other), &current));
        std::fs::remove_file(&other).unwrap();
        assert!(legacy_value_is_replaced(&startup_command(&other), &current));
        assert!(!legacy_value_is_replaced("unrecognized command", &current));
        assert_eq!(startup_executable(&startup_command(&current)), Some(current));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn channel_startup_names_are_distinct_and_only_portable_has_a_legacy_alias() {
        assert_eq!(value_name_for("stable"), "CodexStatus");
        assert_eq!(value_name_for("beta"), "CodexStatusBeta");
        assert_eq!(value_name_for("development"), "CodexStatusDevelopment");
        assert_eq!(value_name_for("portable"), "CodexStatusPortable");
        assert_eq!(legacy_value_name_for("portable"), Some("CodexStatus"));
        assert_eq!(legacy_value_name_for("stable"), None);
    }
}
