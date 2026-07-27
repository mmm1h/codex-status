//! Tiny JSONL fixture server used for manual UI and process-lifecycle tests.
//! It is not part of the CodexStatus binary or release archives.

use std::io::{self, BufRead, Write};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
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
