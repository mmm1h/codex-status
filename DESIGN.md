---
version: alpha
name: CodexStatus Liquid Aurora
description: A compact native quota flyout built from quiet data hierarchy and softly refracted glass.
colors:
  primary: "#10a37f"
  neutral-hero-light: "#202429"
  neutral-hero-dark: "#f2f5f6"
  muted-light: "#6d737c"
  muted-dark: "#9ea6ab"
  glass-light: "#f6f8fb"
  glass-dark: "#06090d"
  aurora-cyan: "#4cd3cf"
  aurora-rose: "#e86799"
  aurora-apricot: "#eb974f"
  warning: "#a96b16"
  critical: "#b94b55"
typography:
  quota:
    fontFamily: "Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "68px"
    fontWeight: 600
    lineHeight: 1
  brand:
    fontFamily: "Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "18px"
    fontWeight: 600
    lineHeight: 1.2
  title:
    fontFamily: "Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "20px"
    fontWeight: 600
    lineHeight: 1.2
  body:
    fontFamily: "Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.35
  label:
    fontFamily: "Segoe UI Variable Text, Segoe UI, sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.35
rounded:
  gauge: "4px"
  control: "9px"
  surface: "12px"
spacing:
  micro: "4px"
  compact: "8px"
  surface: "16px"
  content: "32px"
components:
  glass-surface:
    backgroundColor: "{colors.glass-light}"
    textColor: "{colors.neutral-hero-light}"
    rounded: "{rounded.surface}"
    padding: "16px"
  glass-surface-dark:
    backgroundColor: "{colors.glass-dark}"
    textColor: "{colors.neutral-hero-dark}"
    rounded: "{rounded.surface}"
    padding: "16px"
  refresh-control:
    textColor: "{colors.muted-light}"
    rounded: "{rounded.control}"
    size: "36px"
  refresh-control-dark:
    textColor: "{colors.muted-dark}"
    rounded: "{rounded.control}"
    size: "36px"
  aurora-cyan-pool:
    backgroundColor: "{colors.aurora-cyan}"
    rounded: "{rounded.surface}"
  aurora-rose-pool:
    backgroundColor: "{colors.aurora-rose}"
    rounded: "{rounded.surface}"
  aurora-apricot-pool:
    backgroundColor: "{colors.aurora-apricot}"
    rounded: "{rounded.surface}"
  capacity-gauge:
    backgroundColor: "{colors.primary}"
    rounded: "{rounded.gauge}"
    height: "8px"
  capacity-gauge-warning:
    backgroundColor: "{colors.warning}"
    rounded: "{rounded.gauge}"
    height: "8px"
  capacity-gauge-critical:
    backgroundColor: "{colors.critical}"
    rounded: "{rounded.gauge}"
    height: "8px"
---

# Design System: CodexStatus Liquid Aurora

## Overview

**Creative North Star: "Liquid Aurora"**

CodexStatus should feel like a compact native instrument viewed through a thin, luminous material. Cyan, rose, apricot, and a trace of violet appear as large refracted volumes behind the interface; they never become text effects or decorative outlines. The number remains the clearest object on the surface.

Light mode is clear and softly luminous. Dark mode is transparent black glass with restrained, low-luminance color pools—not a whitened or grey frosted card.

**Key Characteristics:**

- Neutral, number-first information hierarchy.
- Real Acrylic/DirectComposition translucency with an opaque accessibility fallback.
- Large ambient color fields rather than rainbow borders or gradient text.
- Compact native density and unchanged product semantics.

## Colors

The palette combines graphite neutrals with three atmospheric color pools and one semantic status family.

### Primary

- **Codex Healthy:** Used on the percent mark, capacity gauge, and small healthy-status cues.

### Secondary

- **Aurora Cyan:** The coolest and most visible ambient pool, normally entering from the upper-left.
- **Aurora Rose:** A quieter counterweight near the upper-right.
- **Aurora Apricot:** A low warm pool near the lower edge.

### Neutral

- **Neutral Hero:** The large quota number in both themes.
- **Clear Glass / Black Glass:** Theme-specific card materials that preserve backdrop context.
- **Muted Copy:** Labels, timestamps, and supporting facts.

**The Color-Belongs-to-Material Rule.** Aurora color may live behind glass, in the capacity gauge, or in small semantic cues; never place it inside headline or quota-number glyphs.

## Typography

**Display Font:** Segoe UI Variable Text, falling back to Segoe UI.
**Body Font:** Segoe UI Variable Text, falling back to Segoe UI.

**Character:** Native, direct, and highly legible. Tabular figures prevent quota and time values from shifting during refreshes.

### Hierarchy

- **Quota** (semibold, 68px): The primary weekly remaining value; the percent sign drops to 20px and carries semantic color.
- **Title** (semibold, 20px): Reset values and metric values.
- **Brand** (semibold, 18px): CodexStatus in the header.
- **Body** (regular, 14px): Weekly label and pace message.
- **Label** (regular, 12px): Update time, reset metadata, metric labels, and footer.

**The Flat-Type Rule.** Text uses solid fills only—no glow, bloom, gradient fill, or colored outline.

## Layout

The flyout is a fixed 420 × 412 logical-pixel native surface that scales per monitor DPI. A 52px header and 32px footer frame two inset content surfaces. Primary content uses 32px horizontal alignment; surface edges use 16px insets. The quota, gauge, pace, reset timing, and facts preserve the existing scan order and localization behavior.

## Elevation & Depth

Depth comes from Acrylic blur, transparent tonal layering, one soft offset shadow, and a narrow top-edge highlight. The interface avoids stacked glass-on-glass fog: content cards are dark transparent layers in dark mode and clear white layers in light mode, while the Aurora pools remain behind them.

**The Black-Glass Rule.** Dark surfaces must remain visibly black; increasing translucency must never be simulated by adding a milky grey veil.

## Shapes

Content surfaces use 12px corners, the refresh control uses 9px corners, and the 8px capacity gauge uses a 4px full-pill radius. Borders are single-pixel optical separators and never carry the Aurora spectrum.

## Components

### Refresh Control

- **Shape:** Compact rounded square with a 36px hit area and 9px corners.
- **Color:** Neutral translucent fill; the icon follows muted, hover, pressed, and high-contrast roles.
- **Motion:** The native ring arrow rotates only while refresh is active.

### Cards / Containers

- **Corner Style:** Soft native surface corners (12px).
- **Background:** Theme-specific transparent glass with a subtle diagonal sheen.
- **Shadow Strategy:** One ambient offset shadow for separation from the outer material.
- **Border:** A quiet white optical edge, stronger in light mode and restrained in dark mode.

### Capacity Gauge

- **Track:** Static, read-only, 8px high, and intentionally quiet.
- **Fill:** A four-stop low-difference gradient; teal-to-emerald when healthy, amber or red at lower remaining thresholds.
- **Detail:** One restrained inset top highlight. No thumb, endpoint dot, bezel, glow, or inner plastic shadow.

## Do's and Don'ts

### Do:

- **Do** keep the quota number neutral and let the percent mark and gauge carry status.
- **Do** test light, dark, high-contrast, disabled-transparency, and GDI fallback states.
- **Do** keep Aurora volumes broad, soft, and subordinate to data.

### Don't:

- **Don't** apply gradients, glow, bloom, or chromatic outlines to text.
- **Don't** whiten dark glass into grey fog.
- **Don't** add a slider thumb, endpoint dot, or heavy bevel to the read-only capacity gauge.
- **Don't** stack multiple opaque glass cards until the desktop context disappears.
