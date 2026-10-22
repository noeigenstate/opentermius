# OpenTermius desktop architecture (Tauri 2 + React + Rust)

```
native/
  src/            Rust core library `opentermius` (SSH/SFTP, store, keychain, recovery,
                  importers, logs, snippets, server stats, updater, i18n catalog)
  tests/all/      core integration tests (`cargo test --locked`)
  desktop/        Tauri app crate `opentermius-desktop`, binary `opentermius`
    src/main.rs   builder: plugins, managed state, `generate_handler![…]` list
    src/state.rs  `AppState`: `store: Mutex<Store>`, `sessions: SessionManager`, runtime
    src/<feature>.rs   one module per feature: `#[tauri::command]` fns + DTOs
  web/            React 19 + TypeScript + Vite frontend
    src/api/<feature>.ts   typed `invoke`/`listen` wrappers, one file per feature
    src/pages/<Page>.tsx   one page per sidebar section
    src/components/        shared UI (TitleBar, Sidebar, UsageRing, Dialog, …)
    src/i18n.ts            `t("English text")`; Chinese strings in src/i18n/zh/<feature>.ts
    src/styles.css         glass theme tokens (CSS variables) and shared classes
```

## Conventions

- **Commands**: `snake_case` Rust names, called from TS as `invoke("snake_case", { camelCaseArgs })`.
  DTOs use `#[serde(rename_all = "camelCase")]`. Errors are `Result<T, String>` with the
  message already translated by the core (`opentermius::i18n`). Secrets never go to the
  frontend; the backend decrypts and uses them.
- **State**: commands take `tauri::State<'_, AppState>`; the store is locked briefly and
  never across `.await`.
- **Events** (backend → frontend, `app.emit`), payloads camelCase:
  - `session://output` `{ sessionId, data }` — raw terminal bytes, base64.
  - `session://status` `{ sessionId, connected, ended, error, message }`
  - `session://prompt` `{ promptId, sessionId, kind, title, message, fingerprint?, prompts? }`
    with `kind` = `username | password | passphrase | hostKey | keyboardInteractive`;
    answered by `answer_prompt(promptId, accepted, answers: string[])`.
  - `session://stats` `{ sessionId, hostId, cpu, memory, disk }` (numbers or null)
  - `session://closed` `{ sessionId }`
  - `sftp://files`, `sftp://transfer`, `sftp://progress`, `sftp://finished`,
    `sftp://overwrite`, `sftp://delete` — one per `ssh::Event` SFTP variant.
- **Sessions**: `SessionManager` (desktop/src/sessions.rs) owns every `ssh::Session` /
  local terminal, pumps their events to the frontend and exposes
  `send(session_id, ssh::Command)` for other modules (SFTP, snippets).
- **Terminal**: xterm.js renders and emulates; the backend only streams bytes, resizes the
  PTY (`session_resize`) and forwards input (`session_input`).
- **Strings**: every user-visible string in TS goes through `t("…")` with a Chinese entry
  in your feature's own `src/i18n/zh/<feature>.ts` (default-exported map); Rust messages go through `opentermius::i18n` (catalog tests).
- **Style**: reuse the CSS variables in `styles.css`; cards solid white, chrome
  translucent (`.glass`); no new UI libraries without need.

## Verification (every change)

In `native/`: `cargo build --locked` (zero warnings) and `cargo test --locked`.
In `native/web/`: `npm run build` (TypeScript strict + Vite). Follow
`skills/code-quality-refactor/SKILL.md`.

## Packaging and versions

`desktop/tauri.conf.json` is the version source. `scripts/check-versions.mjs`
checks it against `desktop/Cargo.toml`, `Cargo.toml`, `web/package.json` and the
npm lockfile. Packaging and CI run this check before building.

For shipping, build `web/dist` first (`npm ci` then `npm run build` in `web/`),
then run this command from `native/`:

```sh
cargo build --release --locked -p opentermius-desktop --features tauri/custom-protocol
```

The feature embeds the frontend and enables the production asset protocol;
plain Cargo builds use the development URL. `scripts/package.ps1` automates
this sequence and wraps the release executable in the existing NSIS installer.
Tauri's bundler stays disabled to preserve the updater's asset names, HKCU
install identity and `/S /D=<dir>` upgrade contract. Setup supplies WebView2
through Microsoft's Evergreen bootstrapper only when the runtime is missing.
See `native/installer/README.md` (relative to the repository root) for details.

Core tests do not certify desktop feature wiring. In particular the initial
shell does not dispatch `--apply-update`; the updater integration must handle
that before initializing the WebView to preserve the helper/restart flow.
