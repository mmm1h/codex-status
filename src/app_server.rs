use crate::model::{ParseError, QuotaSnapshot, parse_snapshot};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows::Win32::System::Pipes::PeekNamedPipe;
use windows::Win32::System::Threading::CREATE_NO_WINDOW;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);
const ERROR_LIMIT: usize = 240;

#[derive(Debug, thiserror::Error)]
pub enum AppServerError {
    #[error("Codex is not installed or is not available on PATH")]
    CodexNotFound,
    #[error("Node.js is required for this Codex installation")]
    NodeNotFound,
    #[error("Unsupported Codex wrapper: {0}")]
    UnsupportedWrapper(String),
    #[error("Could not start Codex app-server: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("Could not communicate with Codex app-server: {0}")]
    Io(#[source] std::io::Error),
    #[error("Codex app-server did not respond within 8 seconds")]
    Timeout,
    #[error("Codex app-server closed before returning quota data")]
    Closed,
    #[error("Codex app-server rejected {method}: {message}")]
    Rpc { method: &'static str, message: String },
    #[error(transparent)]
    Parse(#[from] ParseError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandSpec {
    program: PathBuf,
    args: Vec<OsString>,
}

#[derive(Debug, Clone)]
pub struct AppServerClient {
    commands: Result<Vec<CommandSpec>, String>,
}

impl AppServerClient {
    pub fn new() -> Self {
        Self { commands: resolve_commands().map_err(|error| error.to_string()) }
    }

    pub fn fetch(&self) -> Result<QuotaSnapshot, AppServerError> {
        let commands = self.commands.as_ref().map_err(|message| {
            if message.contains("Node.js") {
                AppServerError::NodeNotFound
            } else if message.contains("wrapper") {
                AppServerError::UnsupportedWrapper(message.clone())
            } else {
                AppServerError::CodexNotFound
            }
        })?;
        let mut last_spawn_error = None;
        for command in commands {
            match fetch_with_command(command) {
                Err(AppServerError::Spawn(source))
                    if matches!(
                        source.kind(),
                        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::NotFound
                    ) =>
                {
                    last_spawn_error = Some(AppServerError::Spawn(source));
                }
                result => return result,
            }
        }
        Err(last_spawn_error.unwrap_or(AppServerError::CodexNotFound))
    }
}

impl Default for AppServerClient {
    fn default() -> Self {
        Self::new()
    }
}

fn fetch_with_command(command: &CommandSpec) -> Result<QuotaSnapshot, AppServerError> {
    let mut child = spawn(command)?;
    let job = JobGuard::assign(&child).ok();
    let stdout = child.stdout.take().ok_or(AppServerError::Closed)?;
    let stderr = child.stderr.take().ok_or(AppServerError::Closed)?;
    let mut stdin = child.stdin.take().ok_or(AppServerError::Closed)?;
    let (sender, receiver) = mpsc::channel::<String>();
    let stop_readers = Arc::new(AtomicBool::new(false));
    let stdout_stop = Arc::clone(&stop_readers);
    let reader = thread::spawn(move || read_lines(stdout, sender, &stdout_stop));
    let stderr_stop = Arc::clone(&stop_readers);
    let error_reader = thread::spawn(move || read_capped(stderr, 4096, &stderr_stop));

    let result = (|| {
        write_json(
            &mut stdin,
            &json!({
                "method": "initialize",
                "id": 0,
                "params": {"clientInfo": {
                    "name": "codex_status",
                    "title": "CodexStatus",
                    "version": env!("CARGO_PKG_VERSION")
                }}
            }),
        )?;
        let initialize = receive_response(&receiver, 0, "initialize", REQUEST_TIMEOUT)?;
        response_result(&initialize, "initialize")?;

        write_json(&mut stdin, &json!({"method": "initialized", "params": {}}))?;
        write_json(
            &mut stdin,
            &json!({"method": "account/read", "id": 1, "params": {"refreshToken": false}}),
        )?;
        write_json(&mut stdin, &json!({"method": "account/rateLimits/read", "id": 2}))?;

        let responses = receive_many(&receiver, &[1, 2], REQUEST_TIMEOUT)?;
        let account =
            response_result(responses.get(&1).ok_or(AppServerError::Closed)?, "account/read")?;
        let limits = response_result(
            responses.get(&2).ok_or(AppServerError::Closed)?,
            "account/rateLimits/read",
        )?;
        parse_snapshot(account, limits, chrono::Utc::now().timestamp()).map_err(Into::into)
    })();

    drop(stdin);
    terminate(&mut child);
    // Descendants can inherit the app-server's stdout/stderr handles. Close the
    // kill-on-close job, then cancel the polling readers without waiting for EOF.
    drop(job);
    stop_readers.store(true, Ordering::Release);
    let _ = reader.join();
    let stderr = error_reader.join().unwrap_or_default();

    match result {
        Err(AppServerError::Closed) if !stderr.trim().is_empty() => {
            Err(AppServerError::Rpc { method: "app-server", message: sanitize(&stderr) })
        }
        other => other,
    }
}

fn spawn(spec: &CommandSpec) -> Result<Child, AppServerError> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .arg("app-server")
        .arg("--stdio")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW.0);
    command.spawn().map_err(AppServerError::Spawn)
}

fn terminate(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}

fn write_json(writer: &mut impl Write, value: &Value) -> Result<(), AppServerError> {
    writeln!(writer, "{value}").map_err(AppServerError::Io)?;
    writer.flush().map_err(AppServerError::Io)
}

fn receive_response(
    receiver: &mpsc::Receiver<String>,
    id: u64,
    _method: &'static str,
    timeout: Duration,
) -> Result<RpcResponse, AppServerError> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AppServerError::Timeout);
        }
        let line = receiver.recv_timeout(remaining).map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => AppServerError::Timeout,
            mpsc::RecvTimeoutError::Disconnected => AppServerError::Closed,
        })?;
        if let Ok(response) = serde_json::from_str::<RpcResponse>(&line) {
            if response.id == Some(id) {
                return Ok(response);
            }
        }
    }
}

fn receive_many(
    receiver: &mpsc::Receiver<String>,
    ids: &[u64],
    timeout: Duration,
) -> Result<HashMap<u64, RpcResponse>, AppServerError> {
    let deadline = Instant::now() + timeout;
    let mut responses = HashMap::new();
    while ids.iter().any(|id| !responses.contains_key(id)) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AppServerError::Timeout);
        }
        let line = receiver.recv_timeout(remaining).map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => AppServerError::Timeout,
            mpsc::RecvTimeoutError::Disconnected => AppServerError::Closed,
        })?;
        if let Ok(response) = serde_json::from_str::<RpcResponse>(&line) {
            if let Some(id) = response.id.filter(|id| ids.contains(id)) {
                responses.insert(id, response);
            }
        }
    }
    Ok(responses)
}

fn response_result<'a>(
    response: &'a RpcResponse,
    method: &'static str,
) -> Result<&'a Value, AppServerError> {
    if let Some(error) = &response.error {
        return Err(AppServerError::Rpc { method, message: sanitize(&error.message) });
    }
    response.result.as_ref().ok_or(AppServerError::Closed)
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|character| !matches!(character, '\r' | '\n' | '\t'))
        .take(ERROR_LIMIT)
        .collect()
}

fn read_lines(mut stdout: ChildStdout, sender: mpsc::Sender<String>, stop: &AtomicBool) {
    let mut pending = Vec::new();
    loop {
        let stopping = stop.load(Ordering::Acquire);
        let Ok(available) = pipe_bytes_available(&stdout) else {
            break;
        };
        if available == 0 {
            if stopping {
                break;
            }
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        let mut chunk = vec![0; available.min(4_096)];
        let Ok(read) = stdout.read(&mut chunk) else {
            break;
        };
        if read == 0 {
            break;
        }
        pending.extend_from_slice(&chunk[..read]);
        while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<_> = pending.drain(..=newline).collect();
            while line.last().is_some_and(|byte| matches!(byte, b'\r' | b'\n')) {
                line.pop();
            }
            if sender.send(String::from_utf8_lossy(&line).into_owned()).is_err() {
                return;
            }
        }
    }
}

fn read_capped(mut stderr: ChildStderr, limit: usize, stop: &AtomicBool) -> String {
    let mut bytes = Vec::with_capacity(limit);
    loop {
        let stopping = stop.load(Ordering::Acquire);
        let Ok(available) = pipe_bytes_available(&stderr) else {
            break;
        };
        if available == 0 {
            if stopping {
                break;
            }
            thread::sleep(Duration::from_millis(2));
            continue;
        }
        let remaining = limit.saturating_sub(bytes.len());
        let mut chunk = vec![0; available.min(remaining.max(1)).min(4_096)];
        let Ok(read) = stderr.read(&mut chunk) else {
            break;
        };
        if read == 0 {
            break;
        }
        if remaining > 0 {
            bytes.extend_from_slice(&chunk[..read.min(remaining)]);
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn pipe_bytes_available(reader: &impl AsRawHandle) -> windows::core::Result<usize> {
    let mut available = 0;
    unsafe {
        PeekNamedPipe(HANDLE(reader.as_raw_handle()), None, 0, None, Some(&mut available), None)?;
    }
    Ok(available as usize)
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
    id: Option<u64>,
    result: Option<Value>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    message: String,
}

fn resolve_commands() -> Result<Vec<CommandSpec>, AppServerError> {
    if let Some(path) = env::var_os("CODEX_STATUS_CODEX").map(PathBuf::from) {
        return command_from_path(path).map(|command| vec![command]);
    }

    let directories = path_directories();
    let local_executables = local_codex_executables();
    resolve_commands_from(&directories, &local_executables)
}

fn resolve_commands_from(
    directories: &[PathBuf],
    local_executables: &[PathBuf],
) -> Result<Vec<CommandSpec>, AppServerError> {
    let mut commands = Vec::new();
    // Public native installs are preferred. Store package internals can appear on PATH while
    // denying CreateProcess to unpackaged apps, so those candidates are tried last.
    for directory in directories {
        let executable = directory.join("codex.exe");
        if executable.is_file()
            && !executable.to_string_lossy().to_ascii_lowercase().contains("\\windowsapps\\")
        {
            commands.push(CommandSpec { program: executable, args: Vec::new() });
        }
    }
    // Codex Desktop keeps an executable specifically for local app-server integrations outside
    // PATH. These stable per-user locations remain usable when the packaged WindowsApps binary
    // denies CreateProcess to an unpackaged tray application.
    for executable in local_executables {
        if executable.is_file() && !commands.iter().any(|command| command.program == *executable) {
            commands.push(CommandSpec { program: executable.clone(), args: Vec::new() });
        }
    }
    for directory in directories {
        let wrapper = directory.join("codex.cmd");
        if wrapper.is_file() {
            if let Ok(command) = command_from_path(wrapper) {
                commands.push(command);
            }
        }
    }
    for directory in directories {
        let executable = directory.join("codex.exe");
        if executable.is_file()
            && executable.to_string_lossy().to_ascii_lowercase().contains("\\windowsapps\\")
        {
            commands.push(CommandSpec { program: executable, args: Vec::new() });
        }
    }
    if commands.is_empty() { Err(AppServerError::CodexNotFound) } else { Ok(commands) }
}

fn local_codex_executables() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(root) = env::var_os("CODEX_HOME").map(PathBuf::from) {
        roots.push(root);
    }
    if let Some(profile) = env::var_os("USERPROFILE").map(PathBuf::from) {
        let root = profile.join(".codex");
        if !roots.contains(&root) {
            roots.push(root);
        }
    }

    local_codex_executables_from_roots(&roots)
}

fn local_codex_executables_from_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut executables = Vec::new();
    for root in roots {
        executables.push(root.join("plugins").join(".plugin-appserver").join("codex.exe"));
        executables.push(root.join("bin").join("codex.exe"));
        executables.push(root.join(".sandbox-bin").join("codex.exe"));
    }
    executables
}

fn command_from_path(path: PathBuf) -> Result<CommandSpec, AppServerError> {
    let extension = path.extension().and_then(OsStr::to_str).unwrap_or_default();
    if extension.eq_ignore_ascii_case("exe") || extension.is_empty() {
        return Ok(CommandSpec { program: path, args: Vec::new() });
    }
    if extension.eq_ignore_ascii_case("js") {
        let node = find_node().ok_or(AppServerError::NodeNotFound)?;
        return Ok(CommandSpec { program: node, args: vec![path.into_os_string()] });
    }
    if extension.eq_ignore_ascii_case("cmd") {
        let directory = path
            .parent()
            .ok_or_else(|| AppServerError::UnsupportedWrapper(path.display().to_string()))?;
        let script = directory
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin")
            .join("codex.js");
        if !script.is_file() {
            return Err(AppServerError::UnsupportedWrapper(path.display().to_string()));
        }
        let node = directory.join("node.exe");
        let node =
            if node.is_file() { node } else { find_node().ok_or(AppServerError::NodeNotFound)? };
        return Ok(CommandSpec { program: node, args: vec![script.into_os_string()] });
    }
    Err(AppServerError::UnsupportedWrapper(path.display().to_string()))
}

fn find_node() -> Option<PathBuf> {
    path_directories()
        .into_iter()
        .map(|directory| directory.join("node.exe"))
        .find(|path| path.is_file())
}

fn path_directories() -> Vec<PathBuf> {
    env::var_os("PATH").map(|path| env::split_paths(&path).collect()).unwrap_or_default()
}

struct JobGuard(HANDLE);

impl JobGuard {
    fn assign(child: &Child) -> windows::core::Result<Self> {
        unsafe {
            let job = CreateJobObjectW(None, None)?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if let Err(error) = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) {
                let _ = CloseHandle(job);
                return Err(error);
            }
            let process = HANDLE(child.as_raw_handle());
            if let Err(error) = AssignProcessToJobObject(job, process) {
                let _ = CloseHandle(job);
                return Err(error);
            }
            Ok(Self(job))
        }
    }
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root(name: &str) -> PathBuf {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        env::temp_dir().join(format!("codex-status-{name}-{suffix}"))
    }

    #[test]
    fn resolves_npm_wrapper_without_running_a_shell() {
        let directory = root("npm");
        let script = directory
            .join("node_modules")
            .join("@openai")
            .join("codex")
            .join("bin")
            .join("codex.js");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(directory.join("codex.cmd"), "").unwrap();
        fs::write(directory.join("node.exe"), "").unwrap();
        fs::write(&script, "").unwrap();

        let spec = command_from_path(directory.join("codex.cmd")).unwrap();
        assert_eq!(spec.program, directory.join("node.exe"));
        assert_eq!(spec.args, vec![script.into_os_string()]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_unknown_wrapper() {
        let error = command_from_path(PathBuf::from(r"C:\tools\codex.ps1")).unwrap_err();
        assert!(matches!(error, AppServerError::UnsupportedWrapper(_)));
    }

    #[test]
    fn sanitizes_multiline_errors() {
        assert_eq!(sanitize("first\nsecond\tthird"), "firstsecondthird");
    }

    #[test]
    fn puts_inaccessible_store_candidates_after_npm() {
        let directory = root("path with spaces");
        let npm = directory.join("npm");
        let script =
            npm.join("node_modules").join("@openai").join("codex").join("bin").join("codex.js");
        let store = directory.join("WindowsApps").join("OpenAI.Codex");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::create_dir_all(&store).unwrap();
        fs::write(npm.join("codex.cmd"), "").unwrap();
        fs::write(npm.join("node.exe"), "").unwrap();
        fs::write(&script, "").unwrap();
        fs::write(store.join("codex.exe"), "").unwrap();

        let commands = resolve_commands_from(&[store.clone(), npm.clone()], &[]).unwrap();
        assert_eq!(commands[0].program, npm.join("node.exe"));
        assert_eq!(commands[1].program, store.join("codex.exe"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn puts_desktop_app_server_before_inaccessible_store_candidate() {
        let directory = root("desktop-app-server");
        let local = directory.join(".codex").join("plugins").join(".plugin-appserver");
        let store = directory.join("WindowsApps").join("OpenAI.Codex");
        fs::create_dir_all(&local).unwrap();
        fs::create_dir_all(&store).unwrap();
        fs::write(local.join("codex.exe"), "").unwrap();
        fs::write(store.join("codex.exe"), "").unwrap();

        let commands =
            resolve_commands_from(std::slice::from_ref(&store), &[local.join("codex.exe")])
                .unwrap();
        assert_eq!(commands[0].program, local.join("codex.exe"));
        assert_eq!(commands[1].program, store.join("codex.exe"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn desktop_plugin_app_server_precedes_other_per_user_binaries() {
        let root = PathBuf::from(r"C:\Users\tester\.codex");
        assert_eq!(
            local_codex_executables_from_roots(std::slice::from_ref(&root)),
            vec![
                root.join("plugins").join(".plugin-appserver").join("codex.exe"),
                root.join("bin").join("codex.exe"),
                root.join(".sandbox-bin").join("codex.exe"),
            ]
        );
    }

    #[test]
    fn receive_timeout_is_bounded() {
        let (_sender, receiver) = mpsc::channel();
        let error = receive_response(&receiver, 1, "test", Duration::from_millis(1)).unwrap_err();
        assert!(matches!(error, AppServerError::Timeout));
    }
}
