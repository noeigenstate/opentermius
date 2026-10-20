# OpenTermius Visual Design Spec (v1)

> 中文摘要
> 本文档是 OpenTermius 的视觉规范,供多个实现者按页面并行落地。风格对齐官方 Termius 桌面端浅色主题(去掉账号功能)。
> 核心决策:1) 只用一套 Theme 令牌(新增 `Theme.dark` 布尔开关,所有颜色由它派生,Rust 只需改这一个值切换深浅色);2) 卡片统一用白色 `highest` 放在 `base` 灰底上,页眉/侧栏/工具栏用 `high`;3) 所有页面左右留白统一为 20px,标准页面结构 = 56px 页眉(搜索+主按钮) + 45px 工具栏 + 内容区 + 可选 340px 右侧编辑面板;4) 按钮三种尺寸 24/30/36,输入框 36px 圆角 10;5) 禁止使用 std-widgets 的 LineEdit/ComboBox/CheckBox 等(不跟随主题),统一使用 common.slint 里的自定义控件;6) 软件渲染器不依赖 `drop-shadow-*`,用 1px 边框 + 叠层伪阴影;7) 标题栏首个标签宽度固定、文字随当前分区变化,避免跳动并修复非 Hosts 分区时的高亮问题;8) 第 5 节给出按优先级排序的现有截图问题清单。
>
> 所有尺寸为逻辑像素(设计基线截图为 1.5x 缩放,换算时除以 1.5)。

Scope: files `native/ui/common.slint`, `native/ui/main.slint`, `native/ui/pages/*.slint`. This spec changes nothing by itself; implementers apply it page by page. Rules marked **MUST** are not optional. Never invent a color, size or radius that is not listed here; if something is missing, use the nearest token and tell the lead.

---

## 1. Design tokens

### 1.1 Color palette

Existing names are kept. Everything is declared in `Theme` (section 6.1). `Theme.dark` selects the column.

| Theme property | Light | Dark | Usage |
|---|---|---|---|
| `base` | `#edf1f2` | `#1b1e2e` | Page/content background, panel backgrounds behind cards |
| `lowest` | `#282b3d` | `#0f111c` | Title bar background |
| `high` | `#f7f9fa` | `#22263a` | Sidebar, page header, toolbar, table header, dialog body |
| `highest` | `#ffffff` | `#2a2e44` | Cards, host cards, popups, menu, table rows container |
| `border` | `#d5dde0` | `#3a3f58` | Input borders, popup borders, secondary button hover |
| `divider` (new) | `#798c9426` | `#ffffff1a` | 1px separators (replaces hard-coded `#798c941a` / `#d5dde0` lines) |
| `secondary` | `#798c94` | `#8d97b3` | Captions, helper text, placeholder-adjacent text, inactive icons |
| `muted` (new) | `#a4b3ba` | `#5d6483` | Placeholder text, disabled text/icons |
| `text` | `#141729` | `#e8eaf3` | Primary text, icons |
| `accent` | `#2091f6` | `#3b9dff` | Primary buttons, focus border, selected border, links |
| `accent-hover` (new) | `#58adf8` | `#6ab4ff` | Primary button hover |
| `accent-pressed` (new) | `#1479d6` | `#2a85e6` | Primary button pressed |
| `accent-soft` (new) | `#2091f61f` | `#3b9dff33` | Selected row/chip tint, text selection base |
| `on-accent` (new) | `#ffffff` | `#ffffff` | Text/icons on accent or danger fills |
| `danger` (new) | `#f25e61` | `#ff6b6e` | Destructive text/fill, error status text |
| `danger-hover` (new) | `#e04649` | `#ff8386` | Danger fill hover |
| `danger-soft` (new) | `#f25e611f` | `#ff6b6e2e` | Danger badge background, danger flat hover |
| `success` (new) | `#21b568` | `#2dce7a` | Connected dot, success badge text |
| `success-soft` (new) | `#21b5681f` | `#2dce7a2e` | Success badge background |
| `warning` (new) | `#e8920c` | `#f7b84a` | Warning badge text/dot |
| `warning-soft` (new) | `#e8920c1f` | `#f7b84a2e` | Warning badge background |
| `hover` (new) | `#798c9414` | `#ffffff0f` | Hover fill on transparent items (nav, rows, flat buttons) |
| `pressed` (new) | `#798c9426` | `#ffffff1f` | Pressed fill |
| `selected` (new) | `#798c9426` | `#ffffff1f` | Active nav item, active list row |
| `control` (new) | `#798c9426` | `#ffffff1a` | Secondary button fill, segmented track |
| `control-hover` (new) | `#d5dde0` | `#3a3f58` | Secondary button hover (equals old behaviour) |
| `field-bg` (new) | `#ffffff` | `#1b1e2e` | Input fill when NOT inside a card (dialogs, settings rows). Inside cards inputs stay transparent |
| `search-bg` (new) | `#e6ebed` | `#1b1e2e` | Header search field fill |
| `focus-ring` (new) | `#2091f6` | `#3b9dff` | Focus border (1px) of Input/Select/TextArea |
| `scrim` (new) | `#14172999` | `#000000a6` | Modal backdrop |
| `tab-bg` (new) | `#32364a` | `#1c1f30` | Title-bar inactive tab |
| `tab-hover` (new) | `#3b3f55` | `#242842` | Title-bar tab hover |
| `tab-active` (new) | `#454a63` | `#2f3450` | Title-bar active tab |
| `titlebar-fg` (new) | `#8d91a5` | `#8d91a5` | Title-bar icons (menu, +, window buttons) |
| `tile-host` (new) | `#00527c` | `#0b6aa0` | Host / group / key icon tile fill (white glyph). Replaces `#00527c` and `#0f5774` (unify!) |
| `folder` (new) | `#66ccef` | `#66ccef` | Folder glyph |
| `terminal-bg` (new) | `#141729` | `#0f111c` | Terminal area background |
| `terminal-fg` (new) | `#e6e9f2` | `#e6e9f2` | Default terminal foreground (Rust may override per cell) |
| `terminal-selection` (new) | `#2091f640` | `#3b9dff40` | Terminal selection |
| `terminal-cursor` (new) | `#ffffff80` | `#ffffff80` | Terminal cursor block |

Rules:
- Text on `high`/`highest`/`base` is `text` or `secondary` only. Never use `muted` for information the user must read (only placeholders/disabled).
- Disabled = text and icons `muted`; fills become `control` (non-flat) or transparent (flat). No opacity tricks.
- The title bar (`lowest`) and terminal stay dark in both themes.

### 1.2 Typography

Font stays `default-font-family: "sans-serif"` (CJK falls back automatically). Allowed weights: **400, 500, 600, 700 only** (the existing `450` MUST be replaced by 400 because weights between steps are snapped unpredictably by the software renderer). Allowed sizes: 11, 12, 13, 14, 18, 20, 24.

| Style | Size / weight | Color | Where |
|---|---|---|---|
| `page-title` | 18 / 600 | text | Empty-state title, dialog title, settings page title |
| `section-header` | 14 / 600 | text | "Groups", "Hosts", settings group titles, panel card titles ("Address", "General", "Credentials") |
| `body` | 14 / 400 | text | Inputs, list primary text, menu items, buttons use 14 / 500 |
| `body-strong` | 14 / 500 | text | Card titles, buttons, tab labels, table first column |
| `label` | 12 / 500 | secondary | Field labels, table headers |
| `caption` | 12 / 400 | secondary | Card subtitles, helper text, dates, sizes, status bar |
| `micro` | 11 / 500 | per badge | Badges only |
| `mono` | `font-family: "Consolas"`, 13 / 400 | text | Fingerprints, snippet previews, port mappings, log lines (12 for log lines and previews) |
| terminal | Consolas 14 / 400 | terminal-fg | Unchanged (cell metrics depend on it) |

Existing 10px and 11px captions (file size, host address, group count) become 12px caption. Panel titles (`New Host`) use `section-header` (600, not 700).

### 1.3 Spacing scale (px)

`2, 4, 8, 12, 16, 20, 24, 32`. Named usage:
- **Page gutter = 20** (left/right padding of header, toolbar, content and grids; `Theme.gutter`).
- Card inner padding 20 (settings/editor cards) or 12 (host/list cards).
- Gap between cards in a grid 12; gap between stacked cards in a panel 15 (keep) ; between form fields 10; label-to-field 6.
- Section header to its content 12; between sections 24.
- Sidebar item gap 2 (NOT 8), sidebar padding 10.

### 1.4 Radii

| Token | px | Use |
|---|---|---|
| `r-control` | 5 | Buttons M/S, nav items, menu items, icon buttons, table row hover |
| `r-field` | 10 | Inputs, Select, TextArea, search field, buttons L (36px), title-bar tabs, popups, editor cards |
| `r-card` | 15 | Host card, group card, key/snippet cards (60px high cards) |
| `r-dialog` | 12 | Modal dialogs |
| `r-tile` | 10 (36px tile) / 16 (64px empty tile) | Icon tiles |
| pill | 999 | Badges, StatusDot, Toggle |

### 1.5 Elevation (software renderer safe)

Do **not** use `drop-shadow-*` (not reliably rendered by the software renderer; blur is also expensive). Elevation is expressed with border + a fake hard shadow made of stacked translucent rectangles placed *behind* the surface:

| Level | Surfaces | Recipe |
|---|---|---|
| 0 | Cards on `base` | No border, no shadow. White on `#edf1f2` is enough contrast |
| 0-hover | Card hover | `border-width: 1px; border-color: Theme.border` |
| 1 | Selected card | `border-width: 2px; border-color: Theme.accent` (border is drawn inside; no layout shift) |
| 2 | Popup menu, Select list | `border 1px Theme.border` + fake shadow: one Rectangle `x:-1px; y:1px; width:+2px; height:+3px; border-radius: r+1; background: #14172914` and one `x:-2px; y:2px; width:+4px; height:+6px; border-radius: r+2; background: #1417290a` |
| 3 | Dialog | scrim + `border 1px Theme.border` + three stacked fake shadow rects (offsets 1/2/4 px, alphas `14`,`0d`,`08`) |

The shadow rects are implemented once inside the `Popover` / `Dialog` components (section 6.2); pages never draw them.

### 1.6 Icon sizes

Icons are monochrome SVG, always set `colorize`. Sizes: **14** (buttons, nav, inputs, toolbar), **16** (title-bar window buttons, tab icons), **18** (folder rows in tables, title-bar logo), **20** (inside 36px tile), **22** (inside 40px tile), **24** (empty-state glyph inside 64px tile, was 20 in 60px tile). Icon colors: `text` default, `secondary` inactive/helper, `muted` disabled, `on-accent` on primary. Missing icons to add (14px stroke style like the existing set): `check.svg`, `search.svg`, `trash.svg`, `copy.svg`, `play.svg`, `stop.svg`, `edit.svg`, `download.svg`, `menu.svg`, `chevron-right.svg`, `info.svg`, `warning.svg`, `chevron-left.svg`.

---

## 2. Components

All components live in `common.slint`. Heights are fixed with `min-height == max-height`. Hover/pressed backgrounds animate with `animate background { duration: 100ms; }` only (cheap).

### 2.1 Button (`Action`, extended)

Variants (`kind` string property replaces the `primary/flat/secondary` booleans over time; keep the old booleans working):

| Variant | Fill | Text | Hover | Pressed | Disabled |
|---|---|---|---|---|---|
| primary | `accent` | `on-accent` 500 | `accent-hover` | `accent-pressed` | fill `control`, text `muted` |
| secondary (default) | `control` | `text` 500 | `control-hover` | `pressed` | fill `control`, text `muted` |
| flat | transparent | `text` 500 | `hover` | `pressed` | transparent, text `muted` |
| danger | `danger` | `on-accent` 500 | `danger-hover` | `danger-hover` | as primary disabled |
| danger-flat | transparent | `danger` 500 | `danger-soft` | `danger-soft` | transparent, `muted` |

Sizes: **S** 24px high, radius 5, padding 8, text 12; **M** 30px high (default), radius 5, padding 10, text 14; **L** 36px high, radius 10, padding 16, text 14. Icon 14px, gap 6, icon left of label. `min-width = label + padding*2 (+ icon 20)`. Icon-only button: square 30x30 (M) or 24x24 (S), radius 5, glyph 14, flat style. Focus: no ring on buttons (not keyboard-driven yet).

### 2.2 Input (single line)

36px high, radius 10, `border 1px Theme.border`, fill transparent (inside cards) or `field-bg` (flag `filled: true`, used in dialogs/settings). Text 14/400 `text`, padding-x 10 (32 with leading 14px icon at x=10), placeholder `muted`. Focus: border `focus-ring` (1px, color change only). Disabled: border `divider`, text `muted`, fill `control` at lower alpha (`#798c940d`) and **cursor not focusable** (current disabled inputs look identical to enabled ones; MUST differ). Error state: `border-color: Theme.danger`, helper text below in caption `danger`. Compact variant `compact: true` -> 30px high (used for port input and table-inline editing). Header search variant: fill `search-bg`, border `border`, leading `search.svg`, placeholder `secondary`.

### 2.3 TextArea (multi-line, `TextArea` new)

Same border/radius/focus as Input. Padding 10. Fill `field-bg`. `min-height 120px`. Uses `TextInput` with `wrap: word-wrap` inside a `Flickable`/ScrollView. `mono: true` switches to Consolas 13 with line-height implied (used for snippet body and logs filter). Do not use `std-widgets TextEdit`.

### 2.4 Checkbox / Toggle

- **Toggle** (switch): track 36x20 radius pill, off = `border`, on = `accent`; knob 16x16 circle white at x=2 (off) / x=18 (on), animate x 120ms. Disabled: track `divider`, knob `muted`. Always used for boolean settings.
- **Checkbox**: 18x18, radius 5, border 1px `border`; checked = fill `accent` with white `check.svg` 12px; label 14/400 to the right, gap 8. Used only in dialogs/forms where a toggle would be heavy ("Include saved passwords").

### 2.5 Segmented control

Track: height 30, radius 8, fill `control`, padding 2. Segments: equal width (min 72), height 26, radius 6. Selected: fill `highest`, text `text` 500 (dark theme: `highest`), plus nothing else. Unselected: transparent, `secondary` 500; hover `text`. API: `in [string] options; in-out int current-index`. Used for: Theme (Light/Dark/System), forwarding type (Local/Remote/Dynamic), key type, log level.

### 2.6 Dropdown / Select

Closed: identical to Input (36px, radius 10) with value text 14/400 (`muted` when empty = placeholder) and `chevron-down.svg` 14px `secondary` right at 10px. Open: Popover (2.12) positioned directly under the field, width = field width, 4px gap, radius 10, padding 4, items 32px high radius 5 (text 14/400, hover `hover`, current item `selected` + `check.svg` right), max 6.5 items visible (scroll). Closing: click outside or pick item. API: `in [string] options; in-out int current-index; in string placeholder`.

### 2.7 List row (`ListRow`)

Height 56 (two-line: title 14/500 + subtitle 12/400 `secondary`) or 40 (single line). Padding-x 12, gap 12, optional 36px `tile-host` icon tile (radius 10), trailing area (badge, StatusDot, icon buttons). Fill transparent; hover `hover`; selected `accent-soft` + 2px accent bar on the left edge (`x:0; width:2px; height:100%`); row separator 1px `divider` at bottom inset 12px. Trailing icon buttons appear on hover only (same as host card "more").

### 2.8 Card (`Card`) and host card

- **Panel card** (editor/settings): fill `highest`, radius 10, padding 20, content spacing 10. (Replaces `Theme.high` cards in the host editor.)
- **Host/entity card** (60px high, radius 15): fill `highest`, hover border `border` 1px, selected border 2px `accent`; padding 12, spacing 14; 36x36 `tile-host` tile radius 10 with 20px white glyph; title 14/500 elide; subtitle 12/400 `secondary` elide (never 11); trailing 24px "more" icon button visible on hover/selected. Width: grid cell width (see 3.2); the TouchArea covers the whole card except the trailing button.
- **Group card**: identical to host card (same fill `highest`, same tile color, same radius). Subtitle `"N hosts"` (lower-case h, singular "1 host").

### 2.9 Table (`TableHeader`, rows)

Container: fill `highest`, radius 10, 1px `divider` outline, clipped; or full-bleed (SFTP). Header: height 32, fill `high`, text `label` (12/500 `secondary`), bottom border 1px `divider`, cells padded 12 (first 16). Optional sortable chevron 12px after label. Rows: height 40 (SFTP 36, Logs 24), text 14/400 `text` (first column 500), secondary columns 14/400 `secondary` (dates/sizes 13), hover `hover`, selected `accent-soft`, separator 1px `divider`. Column vertical separators only in the SFTP header (existing look), nowhere else. Columns defined as fractions in one place (`property <[float]> cols`) shared by header and rows so they cannot drift.

### 2.10 StatusDot and Badge

- **StatusDot**: 8x8 circle. `kind`: idle `muted`, ok `success`, warn `warning`, error `danger`, busy `accent`. Always accompanied by a text label when meaning is not obvious. 12px gap to label.
- **Badge**: height 20, radius pill, padding-x 8, text `micro` (11/500). Variants: neutral (`control` fill, `secondary` text), accent (`accent-soft`, `accent`), success, warning, danger (each `*-soft` fill, solid text). Used for key type (`ED25519`), forward type (`Local`), log level.

### 2.11 Empty state (`EmptyState`)

Centered column, max width 420, in the content area below header/toolbar (vertical center = center of that area). Stack: 64x64 tile (fill `control`, radius 16, 24px glyph in `text`) -> 20 gap -> title 18/600 `text` -> 8 gap -> description 14/400 `secondary`, centered, word-wrap -> 20 gap -> primary Action size L (min-width 160, radius 10). Optional secondary link-style flat Action under it. Search-no-results variant: no button, title "No matching ..." / description "Try another ...".

### 2.12 Popover / context menu

`Popover` (shared by menu, select list, "more" menus): fill `highest`, radius 10, border 1px `border`, elevation 2, padding 4. Menu item: 32px high, radius 5, padding-x 8, gap 10, 14px icon + text 14/400; hover `hover`; danger item uses `danger` text and `danger-soft` hover; separator = 1px `divider` with 4px vertical margin. Width 200 default (menus), 230 for the app menu. Dismissal: MUST add a full-window transparent `TouchArea` behind every popover (click-away) and Esc closes. Replace the current `NavItem`-based menu items (36px with 13px gap) with `MenuItem`.

### 2.13 Dialog (modal)

Scrim `Theme.scrim` full window with a TouchArea that swallows clicks (click on scrim does NOT dismiss for data-entry dialogs; it dismisses for pickers). Surface: width 440 (confirm), 520 (form), 560 (backup); fill `highest`, radius 12, border 1px `border`, elevation 3, padding 24. Layout: title 18/600 -> 8 -> optional description 14 `secondary` -> 20 -> body (spacing 12) -> 24 -> footer (right aligned, spacing 10, both buttons size L 36px: secondary Cancel, then primary right-most; a destructive confirm uses the danger variant). Max height `window.height - 80`; body scrolls. Esc = Cancel, Enter = primary when focus is in an Input.

### 2.14 Toast / status bar

Status bar (bottom, 28px, fill `high`, 1px `divider` on top, text 12/400 `secondary`, errors `danger`, padding-x 14): MUST set `clip: true` (see fix P0-1). Transient toast (new, optional): bottom-center, 12px above bottom, height 36, padding-x 16, radius 10, fill `lowest` (dark navy), text `on-accent` 14/500, leading 8px StatusDot for ok/error, auto-hide 3s, elevation 2. Only one toast at a time.

### 2.15 Title-bar tabs

Bar height 51, fill `lowest`, padding-x 9. Tabs: **height 32, y = 9** (all tabs, including Home and SFTP; currently 30 and 32 are mixed), radius 10, fill `tab-bg`, hover `tab-hover`, active `tab-active` + 2px `accent` underline (`x: 10px; y: 30px; width: parent.width - 20px; height: 2px; radius 1`). Text 14/500 `#ffffff`, icon 16px. **Widths are fixed and never depend on active state**: Home tab 150 (logo 18 + section name), SFTP tab 96, terminal tabs 160 (min 120, shrink equally when the row overflows). Gap 6. Close "×" on terminal tabs becomes a 20x20 icon button with `close.svg` 12px, visible only when the tab is hovered or active. Status: 8px StatusDot overlaid left of terminal icon is not needed; keep icon colorized `success` when connected else `titlebar-fg`. Hamburger and "+" use `menu.svg`/`plus.svg` 16px `titlebar-fg` in 30x30 hit areas (not text glyphs). Window buttons 30x30 radius 5, hover `#ffffff1a`; the close button hover is `danger` fill with white glyph.

### 2.16 Sidebar nav item (`NavItem`)

Width = sidebar 184 - 20 padding = 164, height 36, radius 5, padding-x 10, icon 14 + gap 13 + text 14. Inactive: transparent, text `text`, icon `secondary`... keep icon `text` (current). Hover: `hover`. **Active: `selected` fill + text weight 600** (the current active/hover look identical at `#798c941a`; active MUST be visibly stronger than hover). Disabled: `muted`. Item spacing 2px (currently 8, which makes a 44px pitch). Settings stays pinned to the bottom with the 10px padding. Divider to content: 1px `divider`.

---

## 3. Page layout templates

Window: title bar 51 / body / optional status bar 28. Body = sidebar 184 + 1px divider + content (+ optional 340px detail panel). Content column coordinates below are relative to the content area.

### 3.1 Standard section page anatomy

```
y=0    PageHeader   56px  fill high    [ search Input (stretch, 36) ] 10 [ primary Action L ]   padding 10 20
y=56   Toolbar      45px  fill high    [ secondary/flat M actions ...........  icon buttons ]   padding 7 20, 1px divider under
y=101  Content      fill base, scroll, padding 20 (gutter), sections spaced 24
                    right: DetailPanel 340px (optional, replaces nothing; content area shrinks)
```

- A section page is a `Rectangle { background: Theme.base; VerticalLayout { PageHeader{} Toolbar{} content } }`. Title is NOT drawn in the header (the sidebar shows where you are); the search placeholder names the thing: `Search keys`, `Search forwarding rules`, `Search snippets`, `Search known hosts`, `Filter logs`.
- Header primary Action (L, 36px): label is the create verb (`New key`, `New rule`, `New snippet`). Pages with nothing to create (Known Hosts, Logs, Settings) hide it. Settings has NO header/toolbar (see 3.8).
- Toolbar left: M buttons (`New ...` is already in header, so toolbar holds secondary actions: Import, Export, Clear, filters). Right: icon buttons 30x30 (flat). Decorative buttons that do nothing MUST be removed or disabled (`muted`) — the Hosts toolbar grid/tag/calendar icons currently look clickable.
- Content states: **loading** nothing (instant, local); **empty** EmptyState (2.11); **no results** EmptyState search variant; **list** per page.

### 3.2 Grid rules (Hosts, Keychain, Snippets cards)

Cell min width 300, gap 12, left/right/top padding 20. `cols = max(1, floor((width - 40 + 12) / (300 + 12)))`, `cell-width = (width - 40 - (cols-1)*12) / cols`, `x = 20 + col*(cell-width+12)`, `y = top + row*(60+12)`. Cards stretch to cell width (no fixed 350). Group cards and host cards share this one grid so their left edges align. Section headers ("Groups", "Hosts") sit at x=20 (the label x currently differs: 20 vs 30), 14/600 `text`, 12px above first row. Content height = computed top + rows*72 + 20 bottom padding.

### 3.3 Right detail/editor panel (340px, like host editor)

Panel x = window.width - 340, below title bar, fill `base`, 1px `divider` on its left edge (new). Header 56px fill `high`: title `section-header` at x=20, then "more" icon button (30) and collapse icon button (30), right padding 14. Body: ScrollView, padding 15, spacing 15, cards = panel cards (`highest`, radius 10, padding 20, spacing 10, card title `section-header` 14/600 with 16px height, 4px extra bottom gap). Footer 76px fill `base`: primary Action L full width (text 14/500) plus, when editing an existing entity, a danger-flat `Delete` M button above it (or in the "more" menu; choose the menu to match Hosts). Footer padding 20/15. Fields in cards use `Field` (label 12 `secondary`, 6px gap, Input). Close on collapse icon / Esc. Opening the panel shrinks the content grid; the content scrolls independently.

### 3.4 Keychain

Header search `Search keys`, primary `New key`. Toolbar: `Import key` (secondary M, icon `arrow-down`), flat `Generate` (disabled until implemented). Content: Section header `Keys` + grid of entity cards: `tile-host` tile with `keychain.svg`, title = label, subtitle = `ED25519 · SHA256:abcd…wxyz` (fingerprint tail in mono 12), trailing Badge (`Key` accent / `Password` neutral). Detail panel cards: **Label**; **Type** (Segmented Password|Key); **Credentials** (Username, Password/Passphrase with eye toggle, Private key path + `Choose file...` secondary M, Public key read-only mono field with `Copy`). Empty state: icon `keychain`, title `No keys yet`, text `Store SSH keys and passwords once and reuse them across hosts.`, button `New key`.

### 3.5 Port Forwarding

Header search, primary `New rule`. Toolbar: Segmented (All | Local | Remote | Dynamic) on the left. Content: Table in a rounded container, columns (fractions): Name 0.24, Type 0.12 (Badge), Mapping 0.34 (mono 13: `localhost:8080 -> db.internal:5432`), Via host 0.16, Status+action 0.14 (StatusDot + `Running`/`Stopped` caption 12, then icon button play/stop, 24px, on the right). Header 32, rows 48 two-line allowed (name 14/500). Detail panel cards: **Type** (Segmented Local|Remote|Dynamic), **Rule** (Label, Bind address, Local port compact, Destination host, Destination port compact — port inputs are 100px wide, 30px compact), **Via** (Select of hosts). Footer primary `Save` / `Save and start`. Empty state: icon `forward`, `No forwarding rules`.

### 3.6 Snippets

Header search, primary `New snippet`. Toolbar: flat `Import` (disabled if not implemented). Content: list (not grid) of ListRows height 56: title 14/500, preview mono 12 `secondary` one-line elide (first line of script), trailing hover buttons `Copy` and `Run` (icon, 24). Detail panel cards: **Label**; **Script** (TextArea mono, min-height 200, grows); footer primary `Save`; `Delete` in more menu. Empty state: icon `snippets`, `No snippets yet`, `Save commands you run often and paste them into any session.`

### 3.7 Known Hosts

Header search `Search known hosts`; no primary action. Toolbar: secondary `Import`, danger-flat `Clear all` (opens confirm Dialog). Content: Table: Host 0.30 (14/500), Key type 0.14 (Badge neutral), Fingerprint 0.38 (mono 12 `secondary`, elide middle is not available -> elide right, full value in a tooltip is out of scope), Added 0.12 (caption), trailing 28px trash icon button visible on hover (`danger` glyph on hover). No detail panel. Deleting one entry is immediate with toast `Removed <host>` and (optional) Undo flat button in the toast. Empty state: icon `known`, `No known hosts`, `Hosts you trust on first connect appear here.`

### 3.8 Logs

Header search `Filter logs`; no primary action. Toolbar: Segmented (All | Info | Warn | Error) left; right: flat `Export` (icon `arrow-up`), danger-flat `Clear`. Content: one container (`highest`, radius 10, margin 20, clipped) holding a scrolling list of rows of height 24, padding-x 12: timestamp mono 12 `secondary` (width 150), level Badge-less colored text mono 12/500 (`secondary`/`warning`/`danger`, width 52), message mono 12 `text` single-line elide. Newest last, auto-scroll to the bottom unless the user scrolled. Empty: icon `history`, `No log entries`.

### 3.9 Settings (grouped form)

No header/toolbar. Page = ScrollView, padding 24 (top) / 20 (x), content column width `min(720, available)` **left aligned at x=20**. First element: page title 18/600 "Settings" (height 28), 24px gap. Then groups, each: group title 13/600 `secondary` (height 20, 8px gap) + Card (`highest`, radius 10, no padding; rows inside). **Row** (`SettingRow`): height 52 (64 with description), padding-x 20; left: label 14/500 + optional description 12/400 `secondary` below; right: control (Toggle / Segmented / Select 200 wide / Input 200 compact / Action M). Rows separated by 1px `divider` inset 20px. Groups, in order: **Appearance** (Theme: Segmented Light|Dark|System; UI scale: Select), **Terminal** (Font size: Select; Cursor: Segmented Block|Bar|Underline; Scrollback lines: compact Input; Copy on select: Toggle), **Security** (Confirm before closing sessions: Toggle; Host key policy: Select Ask|Strict), **Data** (Backup & Restore: Action M `Open...` -> Dialog 3.10; Import hosts; Export hosts), **About** (Version caption, Config folder path mono 12 + `Open` Action S). Gap between groups 24; last group bottom padding 32.

### 3.10 Backup & Restore dialog

Dialog width 560. Title `Backup & Restore`. Description `Back up hosts, keys and settings to an encrypted file.` Body: a Segmented (Back up | Restore) at the top; then one form:
- Back up: **File** (filled Input + `Choose...` secondary M, spacing 8), **Password** (filled Input, secret), **Confirm password**, Checkbox `Include saved passwords and private keys`, helper caption 12 `secondary` under checkbox.
- Restore: **Backup file** (Input + `Choose...`), **Password**, Checkbox `Replace existing data` (default off = merge), warning Badge row `This cannot be undone` (warning-soft box: radius 8, padding 10, 12px text, `warning` 14px icon).
- Inline result row (replaces toast): StatusDot + caption; success `success`, error `danger`, busy `accent` with text `Working...`; fixed 20px high so the dialog does not jump.
Footer: `Cancel` (secondary L), primary `Back up` / `Restore` (L, disabled until the form is valid).

### 3.11 SFTP dual pane

Full-bleed below title bar, two equal panes with a 1px `divider` between. Each pane: **pane header** 44px (`high`): 24px `tile-host` tile (radius 8) + name 14/500 (Local / host name) + stretch + actions (flat M `Filter`, flat M `Actions` with chevron icon) — vertically centered; **path bar** 44px (`high`): flat 24px icon button with a left-pointing chevron (add `chevron-left.svg`) + path Input (compact 30px, filled `field-bg`) vertically centered, padding 10 12; **table**: header 32, rows 36, columns Name 0.44 / Modified 0.24 / Size 0.16 / Kind 0.16, name column has 18px icon (folder `folder`, file `secondary`) at x=16, text at x=44; selected row `accent-soft`. Right pane when not connected: same pane header (title `Remote`, flat `Select host` Action) + EmptyState in the table area (tile 64, title `Connect to host`, text, button `Select host`). Pane widths follow `(width - 1)/2`. Host picker dialog uses Dialog 3.13 (width 440, list rows 52).

### 3.12 Terminal tab bar and terminal page

Title-bar tabs per 2.15. Below, **terminal toolbar** 40px (`high`, 1px `divider` bottom): `Local`/`SSH` Badge neutral at left (not bare text), stretch, flat M `Copy`, flat M `Paste`, secondary M `SFTP`. Terminal area fill `terminal-bg`, padding 10/8 (unchanged metrics). Remote-files side panel (340px): header 44 with title 14/600, `Upload`/`Refresh` S buttons, close icon; path = Input compact (replace std `LineEdit`); rows 36 using the table row spec; busy shows `Loading...` caption 12. Status bar (28) always visible on terminal pages.

---

## 4. Interaction states cheat sheet

| Element | Default | Hover | Pressed | Focus | Disabled |
|---|---|---|---|---|---|
| Nav item | transparent | `hover` | `pressed` | - | `muted` |
| Card | `highest` | + 1px `border` | `highest` | 2px `accent` (selected) | - |
| Row | transparent | `hover` | `pressed` | `accent-soft` (selected) | `muted` text |
| Input | 1px `border` | 1px `border` | - | 1px `focus-ring` | border `divider`, text `muted` |
| Icon button | transparent | `hover` | `pressed` | - | `muted` glyph |
| Tab | `tab-bg` | `tab-hover` | `tab-hover` | - | - |

Cursor: not controllable in Slint's software renderer for all cases; use `mouse-cursor: pointer` on TouchAreas of clickable cards/buttons where supported (`TouchArea.mouse-cursor`), text cursor automatic on inputs.

---

## 5. Prioritized fixes for the current screenshots

Line numbers refer to `native/ui/main.slint` (before the tokens are applied).

**P0 (visible bugs)**
1. Status text clipped at the bottom-left of every page (`hosts.png`: "主机已保存" cut off). Bar at line 387 has `height: 0px` when hidden but its `Text` overflows. Add `clip: true` to that Rectangle, or wrap the Text in `if` the height is > 0.
2. SFTP pane header misalignment (`sftp.png`): `Filter`/`Actions` Actions (lines 405-406) stick to the top of the 50px header because Action has `max-height: 30px` in a `HorizontalLayout`. Wrap the pane header row contents in `VerticalLayout { alignment: center; }` per child or place each Action in a `Rectangle { width; background: transparent; Action { y: (parent.height - self.height)/2; ... } }`. Same bug for the back `‹` button in the path bars (lines 411, 445).
3. Title-bar tab states (lines 160-179, `section-settings.png`): the Home tab says `OpenTermius` and shows the highlight even when Settings/Keychain etc. is the active section, while the SFTP page renames it `Hosts` and changes both tab widths (153->80, 80->139) so tabs jump. Make the Home tab fixed 150px, label = current section title (`Hosts`, `Keychain`, `Port Forwarding`, `Snippets`, `Known Hosts`, `Logs`, `Settings`), active (`tab-active` + 2px accent underline) whenever `hosts-page && !sftp-page`; SFTP tab fixed 96px active only when `sftp-page`; terminal tabs fixed 160px. Unify all tab heights to 32 and `y: 9px` (lines 160/169 use 30/y 11; 179 uses 32/y 7).
4. `+` in the title bar (line 178) and `≡` (line 157) are text glyphs; `+` has no click handler. Give `+` a TouchArea that calls `open-terminal` (local terminal) or remove it; replace both with 16px SVG icons in 30x30 hit boxes.

**P1 (hierarchy and consistency)**
5. Hosts grid alignment (lines 263-317): "Groups" label x=20, "Hosts" label x=30, group cards start at x=16 and are `(grid width/cols) - 8` wide while host cards start at x=30 and are fixed 350. Rebuild with one grid (3.2): x=20, equal cell width. Group cards use the same fill (`highest`) and tile color (`tile-host`) as host cards (currently `high` vs `highest`, `#0f5774` vs `#00527c`). Both section labels become `section-header` 14/600 (currently 13/600 and 14/500). Subtitle 11px -> 12px; `"1 Hosts"` -> `"1 host"`.
6. Three different left edges on one page: search bar x=8 (line 216), toolbar x=12 (line 226), content 16/30. Make every row use the 20px gutter: header `padding: 10px 20px`, toolbar `x: 20px`, content `20px`.
7. Header and toolbar bands share `Theme.high` with no separator, and the body below has no top edge. Add 1px `divider` at the bottom of the toolbar; keep header/toolbar one band (no line between them).
8. Host editor cards (lines 505-542) use `Theme.high` on `base`, which is nearly invisible. Switch to `Theme.highest`, radius 10, padding 20. Add the 1px left divider to the panel (line 487) and the footer top shadow-less divider.
9. Host editor hacks: `Backspace      Default` placeholder with spaces (line 522) and a disabled `Tags` input that looks live (line 521). Replace with a `Select` (`Backspace` label left, value `Default` right) and a visibly disabled Input (2.2). Fix the `SSH on [22] port` row (lines 527-531): text widths 46/27 clip; use auto-width Texts, port Input compact 30px / width 80, vertically centered, label font 14/500.
10. Sidebar spacing (line 200): `spacing: 8px` -> `2px`; active vs hover indistinguishable (NavItem line 68) -> `selected` fill + weight 600 for active, `hover` for hover only.
11. Hosts toolbar right icons (lines 241-247, grid/tag/calendar) are inert and misplace when the editor opens; remove them or render at `muted` with no hover until implemented. The `Serial` disabled button (line 238): keep but `muted`.
12. Replace placeholder section pages (`Text` centered 18px `secondary`) with the templates in section 3 (header + toolbar + EmptyState).
13. Empty state (lines 251-260, 425-433): use the shared `EmptyState` (64px tile radius 16, 24px glyph, title 18/600 not 500, button size L). The SFTP right pane empty state has no header while the connected state has one; always render the 44px pane header.

**P2 (polish)**
14. SFTP table (`FileTableRow`, lines 19-33): header 44px with 14px/500 text is as loud as rows; make header 32px, 12/500 `secondary`, fill `high`; rows 36px; selected `accent-soft`; hover `hover`. Replace the `0.44/0.68/0.84` magic numbers with one `cols` array. Use `Theme.divider` for the vertical lines and the pane split (line 423 uses `#d5dde0`, sidebar uses `#798c941a`).
15. Dialogs (lines 459-581): use `Theme.scrim` (was `#14172980` and `#141729bf`), `Dialog` component (radius 12, border, fake shadow, `highest` fill), footer buttons L. Replace `std-widgets LineEdit` (lines 368, 574) with `Input` (filled). Delete the legacy `settings-open` dialog (line 558) once the Settings page exists.
16. Popup menus (lines 480, 552): hard-coded `white`; add click-away layer, `Popover`, `MenuItem` (32px), danger item for `Remove`. `menu-open` currently only closes by toggling the hamburger.
17. Hard-coded colors to replace with tokens: `#798c941a` -> `divider`/`hover`, `#798c9426` -> `control`, `#d5dde0` -> `border`, `#e6ebed` -> `search-bg`, `#00527c`/`#0f5774` -> `tile-host`, `#66ccef` -> `folder`, `#f25e61` -> `danger`, `#21b568` -> `success`, `#3e4257`/`#32364a` -> `tab-active`/`tab-bg`, `#8d91a5` -> `titlebar-fg`, `#141729` (terminal) -> `terminal-bg`, `#2091f640` -> `terminal-selection`, `#ffffff80` -> `terminal-cursor`, `#a4b3ba` -> `muted`, `#58adf8` -> `accent-hover`, `white` -> `on-accent` or `highest`.
18. `font-weight: 450` (Action secondary, FileTableRow) -> 400; `font-weight: 700` (editor title) -> 600.
19. Terminal toolbar (line 326): bare `Local`/`SSH` text -> neutral Badge; `Action` flat buttons ok. Status bar (line 387) text 11px -> 12px.
20. Sidebar lists terminal sessions again (line 207) although they already exist as title-bar tabs; remove to reduce duplication (optional, ask lead).

---

## 6. Slint implementation notes

### 6.1 Theme global

One bool drives everything. Rust does `app.global::<Theme>().set_dark(true)`.

```slint
export global Theme {
    in-out property <bool> dark: false;
    in-out property <length> gutter: 20px;
    // existing names
    out property <color> base: root.dark ? #1b1e2e : #edf1f2;
    out property <color> lowest: root.dark ? #0f111c : #282b3d;
    out property <color> high: root.dark ? #22263a : #f7f9fa;
    out property <color> highest: root.dark ? #2a2e44 : #ffffff;
    out property <color> border: root.dark ? #3a3f58 : #d5dde0;
    out property <color> secondary: root.dark ? #8d97b3 : #798c94;
    out property <color> text: root.dark ? #e8eaf3 : #141729;
    out property <color> accent: root.dark ? #3b9dff : #2091f6;
    // new names: divider, muted, accent-hover, accent-pressed, accent-soft, on-accent,
    // danger*, success*, warning*, hover, pressed, selected, control, control-hover,
    // field-bg, search-bg, focus-ring, scrim, tab-*, titlebar-fg, tile-host, folder,
    // terminal-*  — same pattern, values from table 1.1
}
```

- Inside a global, `root.dark` refers to the global itself; that is valid. Keep every property `out` except `dark`/`gutter` (`in-out`) so only one switch exists; if the lead prefers Rust to push explicit colors, change `out` to `in-out` and have Rust set them all (not recommended).
- Never copy a hex literal into a page. Pages read `Theme.*` only. Window `background: Theme.base` already follows.
- Status bar and other alpha colors are defined as 8-digit hex tokens; do not combine tokens with `.with-alpha()` in pages.

### 6.2 Components to add to `common.slint` (all `export component`)

| Component | Key API | Notes |
|---|---|---|
| `Action` (extend) | `kind: string` (`primary|secondary|flat|danger|danger-flat`), `size: string` (`s|m|l`), existing props kept | M default; map old `primary`/`flat` booleans into `kind` internally |
| `IconButton` | `icon`, `size: length (30/24)`, `danger: bool`, `clicked` | square, flat, 14px glyph |
| `Input` (extend) | `filled`, `compact`, `invalid`, `leading-icon` | disabled style 2.2 |
| `TextArea` | `text` in-out, `placeholder`, `mono`, `min-height` | built from `TextInput` in `Flickable` |
| `Toggle` | `in-out checked`, `enabled`, `toggled` callback | 36x20 |
| `Checkbox` | `in-out checked`, `text` | 18x18 |
| `Segmented` | `in [string] options`, `in-out int current-index`, `changed` | width = options * 72 min |
| `Select` | `in [string] options`, `in-out int current-index`, `placeholder` | opens `Popover` |
| `Popover` | `in-out open`, content slot via `@children` | 2.12, includes click-away layer + fake shadow |
| `MenuItem` | `text`, `icon`, `danger`, `clicked` | 32px |
| `Card` | `title` (optional), `@children` | `highest`, radius 10, padding 20, spacing 10 |
| `EntityCard` | `title`, `subtitle`, `icon`, `selected`, `clicked`, `double-clicked`, `more-clicked` | the host card (2.8); used by hosts/groups/keychain |
| `ListRow` | `title`, `subtitle`, `selected`, `@children` trailing | 2.7 |
| `TableHeader` | `in [string] labels`, `in [float] cols`, `separators: bool` | shares `cols` with rows |
| `StatusDot` | `kind: string` | 8px |
| `Badge` | `text`, `kind: string` | 20px |
| `PageHeader` | `in-out search`, `placeholder`, `primary-text`, `primary-icon`, callbacks `search-changed`, `primary-clicked`, `show-primary: bool` | 56px band, 20px gutter |
| `PageToolbar` | `@children` left, `right` slot | 45px band + bottom divider |
| `EmptyState` | `icon`, `title`, `description`, `action-text`, `action-visible`, `action-clicked` | 2.11 |
| `DetailPanel` | `title`, `primary-text`, `primary-enabled`, callbacks `close`, `primary-clicked`, `@children` | 340px, 3.3 |
| `SettingRow` | `label`, `description`, `@children` right control | 52/64px |
| `Dialog` | `in-out open`, `title`, `description`, `width`, `confirm-text`, `confirm-kind`, `confirm-enabled`, callbacks `confirm`, `cancel`, `@children` body | 2.13 |
| `Field` (extend) | add `invalid`, `helper` | label 12/500 |

Use `@children` (not `children`) in Slint 1.x components. Each page file exports only its Page and Bridge; shared pieces go in `common.slint` (workers editing different pages must not edit the same components concurrently: agree the component list first, one worker owns `common.slint`).

### 6.3 Gotchas (read before coding)

1. **Absolute elements need explicit geometry.** In a `Rectangle` (no layout), an element with only `x:`/`y:` still inherits full parent width/height and overflows. Whenever you set `x` or `y` also set `width` and `height` (this is why several current Images/Texts need `width: 14px; height: 14px`).
2. **Layout children with `max-height` stick to the top** (SFTP header bug). Center them with `VerticalLayout { alignment: center; ... }` or a `Rectangle` holder with `y: (parent.height - self.height) / 2`.
3. **`Text` needs `vertical-alignment: center`** and a fixed height when placed next to controls; set `overflow: elide` for any user data (host names, paths, fingerprints). CJK strings are common here.
4. **Do not use `std-widgets` `LineEdit`, `ComboBox`, `CheckBox`, `Switch`, `TextEdit`**: they ignore `Theme` and break dark mode. `ScrollView`/`ListView` are acceptable for scrolling only (their scrollbar style cannot be themed). Use `ListView` for lists that can exceed ~100 rows (Logs, Known Hosts, SFTP); plain `for` inside `ScrollView` is fine below that.
5. **Software renderer:** no `drop-shadow-*`, no `opacity` layers over large areas, no `blur`. `clip: true` on rounded rectangles is OK for small/moderate areas; do not clip the whole window. Animations only on `background`/`x` of small elements, <=150ms.
6. **TouchArea layering:** put the card-wide `TouchArea` *before* trailing buttons in source order so the buttons (declared later, on top) receive clicks; otherwise limit its width (as current code does with `width: parent.width - 42px`). A TouchArea over children hides their `has-hover`.
7. **Conditional elements (`if`) inside layouts** change spacing when the condition is false; wrap each in a `Rectangle` of fixed size if the layout must stay stable (e.g. trailing hover icons).
8. **Overlays:** modal and popover layers must be the *last* children of `AppWindow` (z-order = declaration order). Popover click-away layer = full-size transparent `TouchArea` just under the popover.
9. **Colors:** property type `color` ternaries (`dark ? a : b`) work; keep alpha in the hex literal. `colorize` on `Image` takes a `Theme` color and updates on theme switch.
10. **Fonts:** only weights 400/500/600/700; sizes from section 1.2. Do not size by `font-size: 11px` ad hoc.
11. **Names:** Slint properties are kebab-case; keep existing callback/property names in `AppWindow`, because `main.rs` binds them. Page `Bridge` globals stay as scaffolded; designers do not add Rust-facing properties without telling the feature worker.
12. **Resizing:** minimum window 900x600. The content area min width = 900 - 185 - 340 = 375; grids must degrade to one column; tables hide the least important column below 560px (Known Hosts: Added; Forwarding: Via host; Logs: nothing).
13. **Verify visually** after each page: run the app, take a screenshot at 1240x800 (light and, once available, dark) and compare against this spec: gutter 20, header 56, toolbar 45, panel 340.
