use std::io;
use std::path::Path;
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "CodexStatus";

pub fn is_enabled() -> bool {
    std::env::current_exe().is_ok_and(|executable| is_enabled_for(&executable))
}

pub fn is_enabled_for(executable: &Path) -> bool {
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    current_user
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .ok()
        .and_then(|key| key.get_value::<String, _>(VALUE_NAME).ok())
        .is_some_and(|value| command_matches(&value, executable))
}

pub fn enable(executable: &Path) -> io::Result<()> {
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = current_user.create_subkey_with_flags(RUN_KEY, KEY_WRITE)?;
    let command = startup_command(executable);
    key.set_value(VALUE_NAME, &command)
}

pub fn disable() -> io::Result<()> {
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let key = current_user.open_subkey_with_flags(RUN_KEY, KEY_WRITE)?;
    match key.delete_value(VALUE_NAME) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn startup_command(executable: &Path) -> String {
    format!("\"{}\" --background", executable.display())
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
}
