---
name: desktop-ui-design
description: Refine OpenTermius desktop colours, materials, layout and interactive motion, with bounded rendering cost and actual Tauri WebView screenshot validation.
---

# Desktop visual and motion conventions

Read the code-quality and Windows validation skills before changing the UI.
Use the existing React, CSS and icon components; keep connection logic and
configuration formats independent of visual changes.

## Material and colour

- Keep the workspace opaque (`--surface`) so the desktop wallpaper cannot turn
  its background muddy. Use glass for navigation and transient controls.
- Reuse the shared neutral palette and blue accent. `--accent-fill` is darker
  than the accent to give small white button labels sufficient contrast.
- Use green/amber/red for actual state or thresholds, with a text label as well
  as colour. Unknown metrics remain unknown; screenshots must not invent usage.
- Keep shared control styles in `styles.css`, host workspace styles in
  `hosts.css`, and auxiliary workspace styles in `workspaces.css`. Replace
  obsolete rules when moving them instead of stacking conflicting overrides.
- The native shell owns the outer window contour. Do not add a CSS border or
  radius to `.app`: it creates a second outline inside the native frame. On
  Windows, blur plus Tao's undecorated shadow exposes non-client insets; the
  shell disables that shadow and requests DWM corners on Windows 11, or uses
  one native window region on Windows 10. Update that region on size/DPI
  changes and clear it when maximized; Windows owns a successfully set region.
  Inspect native inner/outer bounds as well as WebView screenshots: a browser
  screenshot alone cannot show a faulty non-client edge.

## Interactive motion without idle work

- Use CSS hover/press depth, short transitions and finite entrance animations.
  Avoid pointer-driven React renders, continuous decorative loops, large
  animated blur/shadow areas and permanent `will-change`.
- Resting controls use `transform: none`. Do not retain final transforms with
  `forwards`/`both`: this can keep compositor layers and change the containing
  block for fixed menus. Allow entrance transforms to finish and disappear.
- On loss of focus or visibility, disable finite decorative animation and show
  the final static content. Pausing an entrance at opacity zero hides content.
- Preserve reduced motion, reduced transparency, forced colours, keyboard
  focus and disabled states. Tooltip text must wrap and work inside dialogs.
- Keep terminal instances and keys stable. Use their existing `visible` path
  for fitting and rendering; do not remount them for page transitions.
- Polling UI such as Logs runs only while its page and document are visible.
  Resume with one refresh, avoid overlapping requests and clean up listeners.

## Evidence

- Use `--data-dir=<fixture-directory>` for test profiles. On Windows,
  `dirs::config_dir()` uses known folders; overriding `APPDATA` does not isolate
  the saved connections. Verify fixture identity before any UI action.
- `native/scripts/verify-ui.mjs` exercises an actual Tauri WebView via a
  loopback-only test debugging port and writes screenshots and check results.
  Supply debugging arguments through a temporary `TAURI_CONFIG` for the QA
  executable. Do not commit the override or ship the QA executable.
- Compare the old and new frontend with the same viewport, fixture data,
  backend build profile and WebView runtime. Label limitations and measure idle
  work separately from active transitions. Core tests do not certify the UI.
- Store screenshots and local reports under ignored `native/test-output/`.
  Reports use placeholders for connection metadata and identify their build.
