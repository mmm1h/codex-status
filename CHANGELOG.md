# Changelog

All notable changes to CodexStatus are documented here.

## [Unreleased]

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
- State the projected weekly depletion time on the pace line when usage is clearly ahead of pace, keeping the local-read privacy note in the footer.
- Keep flyout graphics devices available for quick reopening, then release the complete graphics stack after more than three minutes of inactivity.
- Extend the existing `windows` crate feature set for Direct3D 11, DXGI, and DirectComposition without adding a new third-party crate.

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
