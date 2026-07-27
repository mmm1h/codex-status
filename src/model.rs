use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const WEEK_MINUTES: u64 = 7 * 24 * 60;
pub const SESSION_MINUTES: u64 = 5 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub window_minutes: u64,
    pub resets_at: Option<i64>,
}

impl QuotaWindow {
    pub fn display_percent(&self) -> u8 {
        self.remaining_percent.round().clamp(0.0, 100.0) as u8
    }

    pub fn is_cache_valid(&self, now: i64, fetched_at: i64) -> bool {
        match self.resets_at {
            Some(reset) => reset > now,
            None => now.saturating_sub(fetched_at) < 15 * 60,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub plan_type: Option<String>,
    pub reset_credits: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSnapshot {
    pub weekly: Option<QuotaWindow>,
    pub session: Option<QuotaWindow>,
    pub account: AccountSummary,
    pub fetched_at: i64,
}

impl QuotaSnapshot {
    pub fn is_cache_valid(&self, now: i64) -> bool {
        self.weekly.as_ref().is_some_and(|window| window.is_cache_valid(now, self.fetched_at))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshState {
    Loading,
    Live,
    Cached,
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct DisplayState {
    pub snapshot: Option<QuotaSnapshot>,
    pub refresh_state: RefreshState,
    pub error: Option<String>,
}

impl DisplayState {
    pub fn loading(snapshot: Option<QuotaSnapshot>) -> Self {
        let refresh_state =
            if snapshot.is_some() { RefreshState::Cached } else { RefreshState::Loading };
        Self { snapshot, refresh_state, error: None }
    }

    pub fn weekly_percent(&self) -> Option<u8> {
        self.snapshot.as_ref()?.weekly.as_ref().map(QuotaWindow::display_percent)
    }

    pub fn session_percent(&self) -> Option<u8> {
        self.snapshot.as_ref()?.session.as_ref().map(QuotaWindow::display_percent)
    }

    pub fn live(snapshot: QuotaSnapshot) -> Self {
        Self { snapshot: Some(snapshot), refresh_state: RefreshState::Live, error: None }
    }

    pub fn after_error(snapshot: Option<QuotaSnapshot>, error: String, now: i64) -> Self {
        let snapshot = snapshot.filter(|value| value.is_cache_valid(now));
        Self {
            refresh_state: if snapshot.is_some() {
                RefreshState::Cached
            } else {
                RefreshState::Unavailable
            },
            snapshot,
            error: Some(error),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("Codex did not return a rate-limit bucket")]
    MissingBucket,
    #[error("Codex returned malformed rate-limit data: {0}")]
    InvalidData(String),
}

pub fn parse_snapshot(
    account_result: &Value,
    rate_result: &Value,
    fetched_at: i64,
) -> Result<QuotaSnapshot, ParseError> {
    let bucket = select_codex_bucket(rate_result).ok_or(ParseError::MissingBucket)?;
    let mut windows = Vec::with_capacity(2);
    for field in ["primary", "secondary"] {
        if let Some(raw) = bucket.get(field).filter(|value| !value.is_null()) {
            if let Some(window) = parse_window(raw)? {
                windows.push(window);
            }
        }
    }

    let weekly_index =
        windows.iter().position(|window| window.window_minutes == WEEK_MINUTES).or_else(|| {
            windows
                .iter()
                .position(|window| ((6 * 24 * 60)..=(8 * 24 * 60)).contains(&window.window_minutes))
        });
    let weekly = weekly_index.map(|index| windows[index].clone());

    let session = windows
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != weekly_index)
        .find(|(_, window)| window.window_minutes == SESSION_MINUTES)
        .or_else(|| {
            windows
                .iter()
                .enumerate()
                .filter(|(index, _)| Some(*index) != weekly_index)
                .find(|(_, window)| ((4 * 60)..=(6 * 60)).contains(&window.window_minutes))
        })
        .map(|(_, window)| window.clone());

    let plan_type = bucket
        .get("planType")
        .and_then(Value::as_str)
        .or_else(|| account_result.pointer("/account/planType").and_then(Value::as_str))
        .map(str::to_owned);
    let reset_credits =
        rate_result.pointer("/rateLimitResetCredits/availableCount").and_then(Value::as_u64);

    Ok(QuotaSnapshot {
        weekly,
        session,
        account: AccountSummary { plan_type, reset_credits },
        fetched_at,
    })
}

fn select_codex_bucket(rate_result: &Value) -> Option<&Value> {
    if let Some(map) = rate_result.get("rateLimitsByLimitId").and_then(Value::as_object) {
        if let Some(bucket) = map.get("codex") {
            return Some(bucket);
        }
        if let Some(bucket) = map.values().find(|bucket| {
            bucket.get("limitId").and_then(Value::as_str).is_some_and(|id| id == "codex")
        }) {
            return Some(bucket);
        }
    }
    rate_result.get("rateLimits").filter(|value| value.is_object())
}

fn parse_window(value: &Value) -> Result<Option<QuotaWindow>, ParseError> {
    let Some(used_percent) = value.get("usedPercent").and_then(Value::as_f64) else {
        return Ok(None);
    };
    if !used_percent.is_finite() {
        return Err(ParseError::InvalidData("usedPercent is not finite".to_owned()));
    }
    let Some(window_minutes) = value.get("windowDurationMins").and_then(Value::as_u64) else {
        return Ok(None);
    };
    let remaining_percent = (100.0 - used_percent).clamp(0.0, 100.0);
    let resets_at = value.get("resetsAt").and_then(Value::as_i64).filter(|value| *value > 0);
    Ok(Some(QuotaWindow { used_percent, remaining_percent, window_minutes, resets_at }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account() -> Value {
        json!({"account": {"type": "chatgpt", "planType": "plus"}})
    }

    #[test]
    fn parses_weekly_only_bucket() {
        let rate = json!({
            "rateLimits": {
                "limitId": "codex",
                "primary": {"usedPercent": 12.4, "windowDurationMins": 10080, "resetsAt": 2_000_000_000},
                "secondary": null
            }
        });
        let snapshot = parse_snapshot(&account(), &rate, 1_900_000_000).unwrap();
        assert_eq!(snapshot.weekly.unwrap().display_percent(), 88);
        assert!(snapshot.session.is_none());
        assert_eq!(snapshot.account.plan_type.as_deref(), Some("plus"));
    }

    #[test]
    fn finds_weekly_when_primary_and_secondary_are_swapped() {
        let rate = json!({
            "rateLimitsByLimitId": {
                "codex_other": {"limitId": "codex_other", "primary": {"usedPercent": 99, "windowDurationMins": 10080}},
                "codex": {
                    "limitId": "codex",
                    "primary": {"usedPercent": 40, "windowDurationMins": 300},
                    "secondary": {"usedPercent": 25, "windowDurationMins": 10080}
                }
            },
            "rateLimitResetCredits": {"availableCount": 2}
        });
        let snapshot = parse_snapshot(&account(), &rate, 100).unwrap();
        assert_eq!(snapshot.weekly.unwrap().display_percent(), 75);
        assert_eq!(snapshot.session.unwrap().display_percent(), 60);
        assert_eq!(snapshot.account.reset_credits, Some(2));
    }

    #[test]
    fn prefers_quota_plan_when_it_differs_from_the_account_token() {
        let rate = json!({
            "rateLimits": {
                "limitId": "codex",
                "planType": "prolite",
                "primary": {"usedPercent": 12, "windowDurationMins": 10080}
            }
        });
        let snapshot = parse_snapshot(&account(), &rate, 100).unwrap();
        assert_eq!(snapshot.account.plan_type.as_deref(), Some("prolite"));
    }

    #[test]
    fn does_not_mislabel_short_window_as_weekly() {
        let rate = json!({"rateLimits": {
            "primary": {"usedPercent": 5, "windowDurationMins": 60},
            "secondary": {"usedPercent": 10, "windowDurationMins": 300}
        }});
        let snapshot = parse_snapshot(&account(), &rate, 100).unwrap();
        assert!(snapshot.weekly.is_none());
        assert_eq!(snapshot.session.unwrap().display_percent(), 90);
    }

    #[test]
    fn clamps_out_of_range_usage() {
        let high =
            json!({"rateLimits": {"primary": {"usedPercent": 140, "windowDurationMins": 10080}}});
        let low =
            json!({"rateLimits": {"primary": {"usedPercent": -10, "windowDurationMins": 10080}}});
        assert_eq!(
            parse_snapshot(&account(), &high, 0).unwrap().weekly.unwrap().display_percent(),
            0
        );
        assert_eq!(
            parse_snapshot(&account(), &low, 0).unwrap().weekly.unwrap().display_percent(),
            100
        );
    }

    #[test]
    fn invalidates_cached_snapshot_after_reset() {
        let window = QuotaWindow {
            used_percent: 20.0,
            remaining_percent: 80.0,
            window_minutes: WEEK_MINUTES,
            resets_at: Some(500),
        };
        assert!(window.is_cache_valid(499, 100));
        assert!(!window.is_cache_valid(500, 100));
    }

    #[test]
    fn offline_state_keeps_only_unexpired_cache() {
        let snapshot = QuotaSnapshot {
            weekly: Some(QuotaWindow {
                used_percent: 20.0,
                remaining_percent: 80.0,
                window_minutes: WEEK_MINUTES,
                resets_at: Some(500),
            }),
            session: None,
            account: AccountSummary::default(),
            fetched_at: 100,
        };
        let cached = DisplayState::after_error(Some(snapshot.clone()), "offline".to_owned(), 499);
        assert_eq!(cached.refresh_state, RefreshState::Cached);
        let expired = DisplayState::after_error(Some(snapshot), "offline".to_owned(), 500);
        assert_eq!(expired.refresh_state, RefreshState::Unavailable);
        assert!(expired.snapshot.is_none());
    }

    #[test]
    fn tolerates_missing_window_fields_without_inventing_quota() {
        let rate = json!({"rateLimits": {
            "primary": {"windowDurationMins": 10080},
            "secondary": {"usedPercent": 10}
        }});
        let snapshot = parse_snapshot(&account(), &rate, 100).unwrap();
        assert!(snapshot.weekly.is_none());
        assert!(snapshot.session.is_none());
    }
}
