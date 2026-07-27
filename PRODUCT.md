# Product

<!-- impeccable:product-schema 1 -->

## Platform

adaptive

## Users

Primary users are Codex users working on a personal Windows 10 or Windows 11 x64 computer who want to read remaining quota without opening a larger application. This is inferred from the explicit brief and the existing native implementation.

## Product Purpose

CodexStatus keeps the trustworthy remaining Codex quota visible in the Windows notification area and reveals reset timing and supporting account details in one compact flyout. Success means the weekly value is readable immediately, the flyout opens reliably on the first click, and the utility remains unobtrusive while idle.

## Positioning

The product uses the locally installed and signed-in Codex app-server instead of collecting credentials or scraping private endpoints. It combines that privacy boundary with a number-first native tray presentation.

## Operating Context

CodexStatus starts with Windows, stays in the notification area, refreshes on a low-frequency timer, and is normally viewed for only a few seconds at a time. It must work across light, dark, high-contrast, multi-monitor, and per-monitor-DPI Windows environments.

## Capabilities and Constraints

- Native Rust and Win32 application for Windows 10/11 x64.
- Direct2D and DirectWrite may render the flyout; the notification icon remains a standard Win32 icon and GDI remains the rendering fallback.
- The flyout may use a real system backdrop (Desktop Acrylic) and a D3D11-backed Direct2D device context with DirectComposition, so that translucency and blur are genuine rather than simulated. Every such path must degrade to an opaque surface under high contrast, disabled transparency effects, remote sessions, or initialization failure.
- No Electron, WebView, WinUI, WPF, resident web server, token storage, raw-response storage, or telemetry.
- Preserve quota refresh, cache expiry, alert, theme, pin, shortcut, service-check, auto-update, single-instance, and Explorer-recovery behavior unless the user explicitly changes it.
- Low idle CPU and memory usage remain product goals. For the current glass redesign the user has explicitly accepted the added graphics cost up front and deferred its optimization, so visual quality takes precedence while the effect is being built.

## Brand Commitments

- Product name: CodexStatus.
- Interface copy is direct, compact, and available in English and Simplified Chinese.
- The current redesign is explicitly bound to the user-provided Stitch/Gemini reference: soft luminous glass surfaces, a green quota glow, generous rounded panels, and strong numerical hierarchy.

## Evidence on Hand

- Existing native implementation and automated test suite in this repository.
- User-provided light and dark visual references, plus the private Stitch project `CodexStatus Widget Light Mode` accessed for this redesign.
- Real quota, reset, plan, and reset-credit data from the local Codex app-server.
- No official OpenAI artwork exists for plan badges. CodexStatus draws its own plan badges for Free, Go, Plus, and Pro as part of its own visual language; they must never be labelled or described as official OpenAI assets.

## Product Principles

- Read the quota before reading the interface.
- Keep system behavior native, reliable, and reversible.
- Spend visual emphasis on live quota state, not decorative chrome.
- Preserve privacy and make stale or unavailable data explicit.
- Release transient resources when the flyout closes.

## Accessibility & Inclusion

Do not communicate quota state by color alone. Preserve readable text contrast, high-contrast mode, DPI scaling, expanded hit targets, and localized labels.
