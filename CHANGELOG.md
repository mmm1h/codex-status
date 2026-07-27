# Changelog

All notable changes to CodexStatus are documented here.

## [Unreleased]

- Save tray menu settings as a transaction: a failed write now rolls the setting back, skips the runtime side effect, and reports the failure, instead of leaving memory and disk disagreeing until the next restart.
- Validate the startup registry entry against the current executable, so a stale or broken path is no longer reported as enabled and can be repaired by selecting the item again.
- Stop reselecting the current alert threshold from clearing the deduplication state and re-firing an alert that was already delivered.
- Surface previously silent failures when unregistering the global shortcut, saving the shortcut at startup, and redrawing the tray icon on request.
- Rename the releases menu item to describe what it does; it opens the releases page and never checked for a new version.
- Add a "Check for updates now" command that runs the existing verified update path on demand, bypassing only the once-a-day throttle and never the asset, size, or SHA-256 checks.
- Say in the tray tooltip which window a reading actually came from when the selected metric has no data, instead of quietly substituting the weekly value.
- Draw the mutually exclusive menu groups as radio items and leave the on/off items as check marks, matching what each group actually means.
- Let the test notification through Quiet Hours, since its whole purpose is to prove notifications arrive; real quota alerts still respect it.
- Retry a busy clipboard briefly before reporting that a copy failed.

## [0.5.0] - 2026-07-28

- Move the flyout from `ID2D1HwndRenderTarget` to D3D11, an `ID2D1DeviceContext`, and a premultiplied DirectComposition swapchain, with grayscale-antialiased text on transparent surfaces.
- Add live Windows acrylic through `SetWindowCompositionAttribute`, with opaque rendering used for high contrast, disabled transparency effects, Remote Desktop, composition API failures, and graphics initialization or drawing failures; retain GDI as the final fallback.
- Replace simulated card shadows, large-number glow, and progress-endpoint halo with `CLSID_D2D1Shadow` and `CLSID_D2D1GaussianBlur` effects.
- Redesign the flyout as a calm typographic instrument: a neutral tonal ladder in both themes, one hero percentage over a tracked meter, supporting facts set as a single labelled row, and grouping carried by whitespace and elevation instead of rules or containers.
- Drop the large-number glow in light themes, where an emissive halo reads as a rendering artifact rather than as light; dark themes keep only a faint lift.
- Give the refresh control a real button container with distinct rest, hover, and pressed states, and show refresh progress in place.
- Align surface geometry and type to the Windows type ramp and geometry guidance, and enable OpenType tabular figures so refreshed values no longer shift horizontally.
- Project the remaining time at the current burn rate in the footer, tinting the duration with the quota state colour while the pace line keeps only the qualitative judgement.
- Show the expiry date of the reset credit that lapses soonest beneath the credit count.
- Spin the refresh glyph while a refresh is in flight, repainting only the button and stopping the timer as soon as the refresh ends or the flyout hides.
- Enlarge the tray digits and thicken the status rule so the reading survives at 16 px, and move its palette onto the same desaturated green, amber, and red.
- Read the plan identity from the Codex rate-limit bucket before the broader account token, and label plans the way the official Codex CLI does: `prolite` as Pro Lite, `pro` as Pro, `team` as Business, and `business` as Enterprise. Earlier releases labelled organisation plans one tier low.
- Give the footer over to the burn-rate projection and drop the local-read note from it; the privacy boundary is still described in the README and the tray diagnostics.
- Keep flyout graphics devices available for quick reopening, then release the complete graphics stack after more than three minutes of inactivity.
- Extend the existing `windows` crate feature set for Direct3D 11, DXGI, and DirectComposition without adding a new third-party crate.

Known limitations of this release: the new graphics stack was developed and verified on Windows 11 24H2 with two 100% scaling displays. Windows 10, mixed-DPI multi-monitor setups, high contrast toggled at the system level, and software (WARP) rendering were exercised through forced code paths rather than on real hardware. Acrylic depends on an undocumented Windows API, so a future Windows update could disable it; every failure path falls back to opaque rendering and then to GDI, and quota reading is unaffected either way.

## [0.4.0] - 2026-07-27

- Move the details flyout to a lazy Direct2D + DirectWrite renderer with ClearType text, DPI-aware geometry, and the existing GDI path retained as a safe fallback.
- Introduce one restrained native design system for light and dark themes: a focused quota surface, clearer type hierarchy, semantic status colors, and an open metrics grid without stacked mini-cards.
- Add a compact quota trace that combines remaining quota, the current endpoint, the expected time-position marker, and a plain-language pace interpretation.
- Prefer Segoe UI Variable Text through DirectWrite with automatic Segoe UI fallback, keeping Latin, numerals, and localized text in one coherent Windows font family.
- Release graphics resources when the flyout closes and schedule an immediate working-set trim, preserving the low idle footprint despite the higher-fidelity renderer.
- Refresh the documented light and dark screenshots and performance sample for the new interface.

## [0.3.0] - 2026-07-27

- Add independent weekly and five-hour low-quota thresholds, smart pace warnings, recovery alerts, and a test-notification action.
- Add selectable tray values for weekly quota, five-hour quota, or the lower of both; unavailable five-hour data is now omitted from the card.
- Show a time-progress marker and usage-pace interpretation beside the weekly quota bar.
- Add on-demand OpenAI service-status checks with a compact Codex incident badge and a direct link to the official status page.
- Add copyable quota summaries and privacy-safe diagnostics, an optional `Ctrl+Alt+Q` global shortcut, and a pinnable details card.
- Pause quota and service-status timers while Windows is locked or asleep, then refresh once after resume.
- Reorganize the tray menu into compact Refresh, Tray display, Alerts, and Appearance groups.
- Extend cached settings without storing credentials, account identity, raw responses, or usage history.

## [0.2.3] - 2026-07-27

- Generate the Windows manifest and executable version metadata from the Cargo package version so File Explorer reports the installed release correctly.

## [0.2.2] - 2026-07-27

- Use one Segoe UI Variable request throughout the flyout so Latin letters and quota numerals no longer change families between strings; Windows font linking supplies localized glyphs.
- Remove the nested reset panel and three separate metric cards in favor of a calmer split quota surface and one aligned metrics band.
- Refine light and dark semantic colors, dividers, spacing, status accents, and privacy copy for clearer hierarchy and stronger contrast.
- Refresh the documented light and dark screenshots to match the released interface.

## [0.2.1] - 2026-07-27

- Return the process working set to its low idle footprint shortly after the daily WinHTTP update check completes.

## [0.2.0] - 2026-07-27

- Replace the solid block tray badge with transparent, theme-aware Segoe UI quota digits and a restrained one-pixel status rule.
- Fix the tray bitmap orientation that could make a weekly value ending in `2` look like `5`.
- Follow the Windows system theme independently from the app theme so the number stays legible on light and dark taskbars.
- Recompose the flyout around a focused weekly-quota card, a quiet reset panel, three consistent metric cards, and a roomier Fluent-style spacing system.
- Add silent daily updates from verified GitHub Release executables, with SHA-256 digest validation, atomic replacement, and automatic restart.
- Add system, light, and dark flyout theme choices while preserving Windows high-contrast behavior.
- Use Microsoft YaHei UI for Simplified Chinese and Segoe UI Variable for Latin text and quota numerals, with larger supporting type.

## [0.1.1] - 2026-07-27

- Keep the flyout lightweight on systems with third-party input methods by disabling unused text services before window creation.
- Preserve the redesigned readable tray digits and reliable single-click flyout behavior.

## [0.1.0] - 2026-07-26

- Initial public release.
- Weekly quota digits in the standard Windows notification area.
- Native light, dark, high-contrast, and per-monitor-DPI flyout.
- Official Codex app-server quota transport with safe process cleanup.
- Cache expiry, refresh backoff, low-quota alerts, startup control, and single-instance behavior.
- Portable ZIP and per-user Inno Setup installer.
