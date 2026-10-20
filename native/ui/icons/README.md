# UI icons

These SVGs are project-authored geometric UI symbols on a 24 by 24 grid. Slint applies the foreground color for the current surface. The application mark is maintained separately in `assets/logo.svg`.

## Style rules

Every icon in this folder uses the same shape language, so mixed sets stay visually consistent:

- `width="24" height="24" viewBox="0 0 24 24"`
- `fill="none" stroke="#000" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"`
- Geometry inside a 3 to 21 box (about 1.5px optical margin on each side).
- Drawn on the 24px grid and used at 12, 14, 16, 18, 20, 22 or 24 logical pixels (spec 1.6).
  Colours come from `Theme`: `text` by default, `secondary` for helper/inactive icons,
  `muted` for disabled icons and `on-accent` on accent or danger fills.
- Only the app mark (`assets/logo.svg`) is allowed to carry its own colours.

## Index

| Icon | Used for |
| --- | --- |
| `arrow-down.svg`, `arrow-up.svg` | import / export, SFTP transfer direction |
| `calendar.svg` | hosts toolbar (not implemented yet, rendered `muted`) |
| `check.svg` | checkbox, select list current item, menu check |
| `chevron-down.svg`, `chevron-left.svg`, `chevron-right.svg` | selects, breadcrumbs, back button, folder rows |
| `close.svg` | title-bar close button, tab close, dialog close |
| `collapse-right.svg` | detail panel collapse |
| `copy.svg` | copy fingerprint / snippet / terminal selection |
| `download.svg` | download action, empty-state hint |
| `edit.svg` | edit action |
| `eye.svg`, `eye-off.svg` | reveal / hide a secret field (`Input` password toggle) |
| `file.svg`, `folder.svg`, `folder-filled.svg` | file table rows, local/remote panes |
| `forward.svg` | port forwarding nav item and empty state |
| `grid.svg`, `tag.svg` | hosts toolbar (not implemented yet, rendered `muted`) |
| `group.svg` | host group field, group rows |
| `history.svg` | logs nav item and empty state |
| `hosts.svg` | hosts nav item, host tiles, empty state |
| `info.svg` | informational hints |
| `keychain.svg` | keychain nav item, key cards, empty state |
| `known.svg` | known hosts nav item and empty state |
| `local.svg`, `local-filled.svg` | local terminal / local SFTP pane |
| `maximize.svg`, `minimize.svg` | window controls |
| `menu.svg` | title-bar hamburger, overflow menus |
| `more.svg` | entity card "more" button, editor overflow |
| `password.svg` | password field |
| `play.svg`, `stop.svg` | start / stop a port forward, run a snippet |
| `plus.svg` | new host / new key buttons |
| `search.svg` | header search field, filter fields |
| `serial.svg` | serial transport (disabled) |
| `settings.svg` | settings nav item and menu |
| `snippets.svg` | snippets nav item and empty state |
| `terminal.svg` | terminal tabs and nav items |
| `trash.svg` | destructive row / entry removal |
| `user.svg` | username field |
| `vault.svg` | credential hints |
| `warning.svg` | warning badges and the destructive-action hint box |
