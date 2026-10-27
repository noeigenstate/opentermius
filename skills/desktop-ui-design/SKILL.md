---
name: desktop-ui-design
description: Refine OpenTermius desktop colours, materials, layout and interactive motion, with bounded rendering cost and actual Tauri WebView screenshot validation.
---

# Desktop visual and motion conventions

Read the code-quality and Windows validation skills before changing the UI.
Use the existing React, CSS and icon components; keep connection logic and
configuration formats independent of visual changes.

## Material and colour

- Keep the workspace and sidebar opaque (`--surface`, `--panel`): a maximized
  Windows 10 window has no backdrop, so any translucency shows the desktop.
- Colour lives only in the light `:root` block of `styles.css` and the
  `:root[data-theme="dark"]` block after it: the neutral scale `--g0`..`--g9`,
  the semantic tokens (`--accent`, `--accent-fill`, `--success`, `--danger`,
  `--surface`, tone tiles, ...) and the terminal palette `--term-*`. A token
  whose value differs by theme is redefined in the dark block; `data-theme`
  always holds the resolved `light` or `dark`. Rules elsewhere use semantic
  tokens only, with no hex or rgb colour literals (rgba shadows of pure black
  are fine), so a palette change is a token remap. Keep `--accent-fill` dark
  enough for small button labels.
- Icon colour has its own tokens: eight `--icon-*` values (`default`, `secondary`,
  `disabled`, `active`, `hover`, `success`, `warning`, `error`) colour every
  standalone icon. Icons inside filled or inverting buttons inherit the label
  colour, and forced colours remap the tokens to system colours.
- State is never carried by hue alone: status dots are solid and always sit
  next to a text label (connected is bold), usage rings thicken from 70 % and gain a badge from
  90 %, destructive actions use the outlined `btn-danger`. Unknown metrics
  remain unknown; screenshots must not invent usage.
- Keep shared control styles in `styles.css`, host workspace styles in
  `hosts.css`, and auxiliary workspace styles in `workspaces.css`. Replace
  obsolete rules when moving them instead of stacking conflicting overrides.
- Windows 11 uses DWM for the outer contour. Windows 10 uses no GDI region
  (regions cannot anti-alias), no native blur and a fully transparent native
  background; CSS rounds `.app` via `data-window-shape="round"` with one
  hairline border. Maximized/fullscreen windows use `"square"`; DWM windows
  also use `"square"` so CSS adds no second outline. Keep the undecorated
  native shadow disabled and check inner/outer bounds as well as screen
  captures: WebView screenshots alone cannot show a faulty native edge.

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
- xterm's FitAddon sizes the grid from the computed height and width of the
  element passed to `open()`. Under the global `border-box` these include its
  padding, which adds rows and columns that `overflow: hidden` then clips. Keep
  `.xterm-container` `content-box`, and check after a window resize that the
  `.xterm-screen` bottom stays inside the container's padding box.
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
