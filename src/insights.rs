use crate::model::QuotaWindow;
use serde::{Deserialize, Serialize};

/// Usage must lead elapsed time by at least this many percentage points before
/// it is described as clearly ahead of pace.
pub const DEFAULT_AHEAD_MARGIN_PERCENT: f64 = 5.0;

/// The normalized, side-effect-free interpretation of one quota window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowInsight {
    /// Start of the current window, as a Unix timestamp.
    pub window_start_at: Option<i64>,
    /// Validated reset time for the current window, as a Unix timestamp.
    pub reset_at: Option<i64>,
    /// Percentage of the window's wall-clock duration that has elapsed.
    pub elapsed_percent: Option<f64>,
    /// Normalized usage. Invalid values fall back to `remaining_percent`.
    pub used_percent: Option<f64>,
    /// Normalized remaining quota, kept complementary to `used_percent`.
    pub remaining_percent: Option<f64>,
    /// `used_percent - elapsed_percent`, in percentage points.
    pub pace_delta_percent: Option<f64>,
    /// Whether usage is clearly ahead of a linear, even-consumption pace.
    pub is_ahead_of_pace: bool,
    /// Estimated Unix timestamp at which quota reaches 100% usage.
    pub projected_exhaustion_at: Option<i64>,
    /// Whether the linear projection reaches exhaustion before the reset.
    pub likely_exhaust_before_reset: bool,
}

/// Analyze a window using [`DEFAULT_AHEAD_MARGIN_PERCENT`].
pub fn analyze_window(window: &QuotaWindow, now: i64) -> WindowInsight {
    analyze_window_with_margin(window, now, DEFAULT_AHEAD_MARGIN_PERCENT)
}

/// Analyze a window with a caller-provided pace margin.
///
/// Timing and projection fields remain unavailable when the reset is missing,
/// expired, overflows, or describes a window that has not started yet. Quota
/// percentages are still normalized independently of timing.
pub fn analyze_window_with_margin(
    window: &QuotaWindow,
    now: i64,
    ahead_margin_percent: f64,
) -> WindowInsight {
    let (used_percent, remaining_percent) = normalized_percentages(window);
    let timing = valid_timing(window, now);
    let window_start_at = timing.map(|value| value.start_at);
    let reset_at = timing.map(|value| value.reset_at);
    let elapsed_percent = timing.map(|value| value.elapsed_percent);
    let pace_delta_percent =
        used_percent.zip(elapsed_percent).map(|(used, elapsed)| used - elapsed);
    let margin = if ahead_margin_percent.is_finite() {
        ahead_margin_percent.max(0.0)
    } else {
        DEFAULT_AHEAD_MARGIN_PERCENT
    };
    let is_ahead_of_pace = pace_delta_percent.is_some_and(|delta| delta >= margin);

    let projected_exhaustion_at = timing.and_then(|value| {
        project_exhaustion_at(value.start_at, value.elapsed_seconds, used_percent?)
    });
    let likely_exhaust_before_reset =
        projected_exhaustion_at.zip(reset_at).is_some_and(|(exhaustion, reset)| exhaustion < reset);

    WindowInsight {
        window_start_at,
        reset_at,
        elapsed_percent,
        used_percent,
        remaining_percent,
        pace_delta_percent,
        is_ahead_of_pace,
        projected_exhaustion_at,
        likely_exhaust_before_reset,
    }
}

/// Identifies a quota cycle without retaining account or response data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaCycle {
    pub resets_at: i64,
    pub window_minutes: u64,
}

/// Return a stable identity only for a currently valid quota cycle.
pub fn current_cycle(window: &QuotaWindow, now: i64) -> Option<QuotaCycle> {
    valid_timing(window, now).map(|timing| QuotaCycle {
        resets_at: timing.reset_at,
        window_minutes: window.window_minutes,
    })
}

/// The two independently tracked quota windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuotaKind {
    Session,
    Weekly,
}

/// Minimal state required to suppress duplicate alerts for one window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WindowAlertState {
    pub observed_cycle: Option<QuotaCycle>,
    pub low_alerted_cycle: Option<QuotaCycle>,
    pub depleted: bool,
}

/// Independent notification memory for the session and weekly windows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AlertTracker {
    pub session: WindowAlertState,
    pub weekly: WindowAlertState,
}

impl AlertTracker {
    pub const fn for_kind(self, kind: QuotaKind) -> WindowAlertState {
        match kind {
            QuotaKind::Session => self.session,
            QuotaKind::Weekly => self.weekly,
        }
    }

    pub const fn with_kind(mut self, kind: QuotaKind, state: WindowAlertState) -> Self {
        match kind {
            QuotaKind::Session => self.session = state,
            QuotaKind::Weekly => self.weekly = state,
        }
        self
    }
}

/// Pure output of one alert evaluation. The caller decides whether and how to
/// display a notification, then persists `tracker` if desired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertDecision {
    pub tracker: AlertTracker,
    pub should_notify_low: bool,
    pub should_notify_recovered: bool,
    pub is_new_cycle: bool,
}

/// Evaluate low-quota, recovery, and cycle-transition events for one window.
///
/// A missing or invalid reset cannot identify a cycle reliably, so it produces
/// no event and leaves the tracker untouched. A low alert is emitted at most
/// once per cycle, independently for session and weekly windows. Recovery is a
/// transition from a displayed 0% remaining value to a positive value.
pub fn evaluate_alerts(
    tracker: AlertTracker,
    kind: QuotaKind,
    window: Option<&QuotaWindow>,
    threshold: Option<u8>,
    now: i64,
) -> AlertDecision {
    let unchanged = || AlertDecision {
        tracker,
        should_notify_low: false,
        should_notify_recovered: false,
        is_new_cycle: false,
    };
    let Some(window) = window else {
        return unchanged();
    };
    let Some(cycle) = current_cycle(window, now) else {
        return unchanged();
    };
    let Some(remaining) = normalized_percentages(window).1 else {
        return unchanged();
    };

    let display_remaining = remaining.round().clamp(0.0, 100.0) as u8;
    let previous = tracker.for_kind(kind);
    let is_new_cycle = previous.observed_cycle.is_some_and(|old| old != cycle);
    let depleted = display_remaining == 0;
    let should_notify_recovered = previous.depleted && !depleted;
    let should_notify_low =
        threshold.filter(|value| *value <= 100).is_some_and(|value| display_remaining <= value)
            && previous.low_alerted_cycle != Some(cycle);

    let next = WindowAlertState {
        observed_cycle: Some(cycle),
        low_alerted_cycle: if should_notify_low { Some(cycle) } else { previous.low_alerted_cycle },
        depleted,
    };

    AlertDecision {
        tracker: tracker.with_kind(kind, next),
        should_notify_low,
        should_notify_recovered,
        is_new_cycle,
    }
}

#[derive(Debug, Clone, Copy)]
struct ValidTiming {
    start_at: i64,
    reset_at: i64,
    elapsed_seconds: i64,
    elapsed_percent: f64,
}

fn valid_timing(window: &QuotaWindow, now: i64) -> Option<ValidTiming> {
    let reset_at = window.resets_at.filter(|reset| *reset > now)?;
    let duration_seconds = window.window_minutes.checked_mul(60)?;
    if duration_seconds == 0 {
        return None;
    }
    let duration_seconds = i64::try_from(duration_seconds).ok()?;
    let start_at = reset_at.checked_sub(duration_seconds)?;
    let elapsed_seconds = now.checked_sub(start_at)?;
    if elapsed_seconds < 0 || elapsed_seconds > duration_seconds {
        return None;
    }
    let elapsed_percent =
        (elapsed_seconds as f64 / duration_seconds as f64 * 100.0).clamp(0.0, 100.0);
    Some(ValidTiming { start_at, reset_at, elapsed_seconds, elapsed_percent })
}

fn normalized_percentages(window: &QuotaWindow) -> (Option<f64>, Option<f64>) {
    let used = if window.used_percent.is_finite() {
        Some(window.used_percent.clamp(0.0, 100.0))
    } else if window.remaining_percent.is_finite() {
        Some(100.0 - window.remaining_percent.clamp(0.0, 100.0))
    } else {
        None
    };
    (used, used.map(|value| 100.0 - value))
}

fn project_exhaustion_at(start_at: i64, elapsed_seconds: i64, used_percent: f64) -> Option<i64> {
    if used_percent <= 0.0 {
        return None;
    }
    if elapsed_seconds == 0 {
        return (used_percent >= 100.0).then_some(start_at);
    }

    let projected_duration = elapsed_seconds as f64 * 100.0 / used_percent;
    if !projected_duration.is_finite() || projected_duration > i64::MAX as f64 {
        return None;
    }
    start_at.checked_add(projected_duration.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(used_percent: f64, window_minutes: u64, resets_at: Option<i64>) -> QuotaWindow {
        QuotaWindow {
            used_percent,
            remaining_percent: 100.0 - used_percent,
            window_minutes,
            resets_at,
        }
    }

    #[test]
    fn calculates_pace_and_exhaustion_before_reset() {
        let insight = analyze_window(&window(75.0, 100, Some(10_000)), 7_000);

        assert_eq!(insight.window_start_at, Some(4_000));
        assert_eq!(insight.reset_at, Some(10_000));
        assert_eq!(insight.elapsed_percent, Some(50.0));
        assert_eq!(insight.used_percent, Some(75.0));
        assert_eq!(insight.remaining_percent, Some(25.0));
        assert_eq!(insight.pace_delta_percent, Some(25.0));
        assert!(insight.is_ahead_of_pace);
        assert_eq!(insight.projected_exhaustion_at, Some(8_000));
        assert!(insight.likely_exhaust_before_reset);
    }

    #[test]
    fn does_not_warn_when_projection_reaches_past_reset() {
        let insight = analyze_window(&window(25.0, 100, Some(10_000)), 7_000);

        assert_eq!(insight.elapsed_percent, Some(50.0));
        assert_eq!(insight.projected_exhaustion_at, Some(16_000));
        assert!(!insight.is_ahead_of_pace);
        assert!(!insight.likely_exhaust_before_reset);
    }

    #[test]
    fn custom_margin_is_clamped_and_non_finite_margin_uses_default() {
        let quota = window(54.0, 100, Some(10_000));
        assert!(analyze_window_with_margin(&quota, 7_000, -10.0).is_ahead_of_pace);
        assert!(!analyze_window_with_margin(&quota, 7_000, f64::NAN).is_ahead_of_pace);
        assert!(analyze_window_with_margin(&quota, 7_000, 4.0).is_ahead_of_pace);
    }

    #[test]
    fn falls_back_to_remaining_and_clamps_abnormal_values() {
        let mut quota = window(f64::NAN, 100, Some(10_000));
        quota.remaining_percent = 30.0;
        let fallback = analyze_window(&quota, 7_000);
        assert_eq!(fallback.used_percent, Some(70.0));
        assert_eq!(fallback.remaining_percent, Some(30.0));

        let high = analyze_window(&window(140.0, 100, Some(10_000)), 7_000);
        assert_eq!(high.used_percent, Some(100.0));
        assert_eq!(high.remaining_percent, Some(0.0));

        let low = analyze_window(&window(-20.0, 100, Some(10_000)), 7_000);
        assert_eq!(low.used_percent, Some(0.0));
        assert_eq!(low.remaining_percent, Some(100.0));
        assert_eq!(low.projected_exhaustion_at, None);
    }

    #[test]
    fn rejects_percentages_when_both_sources_are_non_finite() {
        let quota = QuotaWindow {
            used_percent: f64::NAN,
            remaining_percent: f64::INFINITY,
            window_minutes: 100,
            resets_at: Some(10_000),
        };
        let insight = analyze_window(&quota, 7_000);
        assert_eq!(insight.used_percent, None);
        assert_eq!(insight.remaining_percent, None);
        assert_eq!(insight.pace_delta_percent, None);
        assert_eq!(insight.projected_exhaustion_at, None);
        assert!(!insight.is_ahead_of_pace);
        assert!(!insight.likely_exhaust_before_reset);
    }

    #[test]
    fn missing_expired_and_future_windows_have_no_timing_prediction() {
        for quota in [
            window(80.0, 100, None),
            window(80.0, 100, Some(7_000)),
            window(80.0, 100, Some(20_000)),
            window(80.0, 0, Some(10_000)),
        ] {
            let insight = analyze_window(&quota, 7_000);
            assert_eq!(insight.window_start_at, None);
            assert_eq!(insight.reset_at, None);
            assert_eq!(insight.elapsed_percent, None);
            assert_eq!(insight.projected_exhaustion_at, None);
            assert!(!insight.is_ahead_of_pace);
            assert!(!insight.likely_exhaust_before_reset);
        }
    }

    #[test]
    fn rejects_duration_overflow() {
        let quota = window(50.0, u64::MAX, Some(i64::MAX));
        assert_eq!(analyze_window(&quota, 0).window_start_at, None);
        assert_eq!(current_cycle(&quota, 0), None);
    }

    #[test]
    fn already_exhausted_at_cycle_start_projects_immediately() {
        let insight = analyze_window(&window(100.0, 100, Some(10_000)), 4_000);
        assert_eq!(insight.projected_exhaustion_at, Some(4_000));
        assert!(insight.likely_exhaust_before_reset);
    }

    #[test]
    fn tiny_usage_that_overflows_projection_is_ignored() {
        let insight = analyze_window(&window(f64::MIN_POSITIVE, 100, Some(10_000)), 7_000);
        assert_eq!(insight.projected_exhaustion_at, None);
        assert!(!insight.likely_exhaust_before_reset);
    }

    #[test]
    fn low_alert_fires_once_per_cycle() {
        let quota = window(81.0, 100, Some(10_000));
        let first = evaluate_alerts(
            AlertTracker::default(),
            QuotaKind::Weekly,
            Some(&quota),
            Some(20),
            7_000,
        );
        assert!(first.should_notify_low);
        assert!(!first.is_new_cycle);

        let repeated =
            evaluate_alerts(first.tracker, QuotaKind::Weekly, Some(&quota), Some(20), 7_100);
        assert!(!repeated.should_notify_low);
        assert!(!repeated.should_notify_recovered);
        assert!(!repeated.is_new_cycle);
    }

    #[test]
    fn threshold_uses_the_same_rounded_value_as_the_tray() {
        let quota = window(79.6, 100, Some(10_000));
        let decision = evaluate_alerts(
            AlertTracker::default(),
            QuotaKind::Weekly,
            Some(&quota),
            Some(20),
            7_000,
        );
        assert!(decision.should_notify_low);
    }

    #[test]
    fn disabled_or_invalid_threshold_does_not_alert() {
        let quota = window(100.0, 100, Some(10_000));
        for threshold in [None, Some(101), Some(u8::MAX)] {
            let decision = evaluate_alerts(
                AlertTracker::default(),
                QuotaKind::Weekly,
                Some(&quota),
                threshold,
                7_000,
            );
            assert!(!decision.should_notify_low);
        }
    }

    #[test]
    fn a_new_cycle_rearms_low_alerts() {
        let old = window(90.0, 100, Some(10_000));
        let first = evaluate_alerts(
            AlertTracker::default(),
            QuotaKind::Weekly,
            Some(&old),
            Some(20),
            7_000,
        );
        let new = window(85.0, 100, Some(16_000));
        let next = evaluate_alerts(first.tracker, QuotaKind::Weekly, Some(&new), Some(20), 10_100);

        assert!(next.is_new_cycle);
        assert!(next.should_notify_low);
    }

    #[test]
    fn recovery_is_a_depleted_to_positive_transition() {
        let empty = window(100.0, 100, Some(10_000));
        let depleted =
            evaluate_alerts(AlertTracker::default(), QuotaKind::Weekly, Some(&empty), None, 7_000);
        assert!(!depleted.should_notify_recovered);
        assert!(depleted.tracker.weekly.depleted);

        let restored = window(5.0, 100, Some(16_000));
        let recovered =
            evaluate_alerts(depleted.tracker, QuotaKind::Weekly, Some(&restored), None, 10_100);
        assert!(recovered.is_new_cycle);
        assert!(recovered.should_notify_recovered);
        assert!(!recovered.tracker.weekly.depleted);

        let repeated =
            evaluate_alerts(recovered.tracker, QuotaKind::Weekly, Some(&restored), None, 10_200);
        assert!(!repeated.should_notify_recovered);
    }

    #[test]
    fn session_and_weekly_trackers_are_independent() {
        let quota = window(95.0, 100, Some(10_000));
        let weekly = evaluate_alerts(
            AlertTracker::default(),
            QuotaKind::Weekly,
            Some(&quota),
            Some(10),
            7_000,
        );
        assert!(weekly.should_notify_low);
        assert_eq!(weekly.tracker.session, WindowAlertState::default());

        let session =
            evaluate_alerts(weekly.tracker, QuotaKind::Session, Some(&quota), Some(10), 7_000);
        assert!(session.should_notify_low);
        assert_eq!(session.tracker.weekly, weekly.tracker.weekly);
    }

    #[test]
    fn absent_or_unidentifiable_window_leaves_state_untouched() {
        let initial = AlertTracker {
            weekly: WindowAlertState {
                observed_cycle: Some(QuotaCycle { resets_at: 10_000, window_minutes: 100 }),
                low_alerted_cycle: Some(QuotaCycle { resets_at: 10_000, window_minutes: 100 }),
                depleted: true,
            },
            ..AlertTracker::default()
        };
        let stale = window(10.0, 100, Some(7_000));
        for quota in [None, Some(&stale)] {
            let decision = evaluate_alerts(initial, QuotaKind::Weekly, quota, Some(20), 7_000);
            assert_eq!(decision.tracker, initial);
            assert!(!decision.should_notify_low);
            assert!(!decision.should_notify_recovered);
            assert!(!decision.is_new_cycle);
        }
    }
}
