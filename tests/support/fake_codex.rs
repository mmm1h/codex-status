//! Tiny JSONL fixture server used for manual UI and process-lifecycle tests.
//! It is not part of the CodexStatus binary or release archives.

use std::io::{self, BufRead, Write};
use std::process::Command;
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    if std::env::args().any(|argument| argument == "--hold-inherited-pipes") {
        thread::sleep(Duration::from_secs(60));
        return;
    }

    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let weekly_remaining = std::env::var("CODEX_STATUS_FAKE_WEEKLY_REMAINING")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .unwrap_or(62)
        .min(100);
    let delay_ms = std::env::var("CODEX_STATUS_FAKE_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let inherited_pipe_stall =
        std::env::var_os("CODEX_STATUS_FAKE_INHERITED_PIPE_STALL").is_some();
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines().map_while(Result::ok) {
        if line.contains("\"id\":0") {
            writeln!(stdout, r#"{{"id":0,"result":{{}}}}"#).unwrap();
        } else if line.contains("\"id\":1") {
            writeln!(
                stdout,
                r#"{{"id":1,"result":{{"account":{{"type":"chatgpt","planType":"pro"}}}}}}"#
            )
            .unwrap();
        } else if line.contains("\"id\":2") {
            if inherited_pipe_stall {
                let _ = Command::new(std::env::current_exe().unwrap())
                    .arg("--hold-inherited-pipes")
                    .spawn();
                return;
            }
            thread::sleep(Duration::from_millis(delay_ms));
            writeln!(
                stdout,
                r#"{{"id":2,"result":{{"rateLimitsByLimitId":{{"codex":{{"limitId":"codex","primary":{{"usedPercent":27,"windowDurationMins":300,"resetsAt":{}}},"secondary":{{"usedPercent":{},"windowDurationMins":10080,"resetsAt":{}}}}}}},"rateLimitResetCredits":{{"availableCount":2,"credits":[{{"expiresAt":{},"status":"available"}},{{"expiresAt":null,"status":"available"}},{{"expiresAt":{},"status":"redeemed"}}]}}}}}}"#,
                now + 3 * 60 * 60,
                100 - weekly_remaining,
                now + 6 * 24 * 60 * 60 + 7 * 60 * 60,
                now + 5 * 24 * 60 * 60,
                now + 24 * 60 * 60,
            )
            .unwrap();
        }
        stdout.flush().unwrap();
    }
}
