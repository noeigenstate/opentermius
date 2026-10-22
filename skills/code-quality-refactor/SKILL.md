---
name: code-quality-refactor
description: "Mandatory rules for refactoring, cleanup or code-quality review of OpenTermius (Tauri, React, TypeScript and Rust in native/): remove redundancy, lower complexity, preserve behaviour and interfaces, and verify each phase with existing tests."
---

# Code quality and refactoring rules

Goal: less redundancy, lower cognitive complexity, better readability, maintainability and efficiency — without changing what the program does.

## What to fix

1. **Redundant code** — duplicated logic, pointless intermediate variables, wrapper functions that only forward, boilerplate. Merge copies into one helper only when the copies really are the same concept.
2. **Control flow** — flatten nested `if`/`else` with early returns, `let … else`, `?`, `match` and `if let`; replace flag variables with expressions.
3. **Concise code** — shorten long expressions and functions while keeping them readable; prefer iterator chains, `Option`/`Result` combinators and standard-library helpers over hand-written loops when they read better.
4. **Structure** — split functions and modules that do several jobs; extract shared logic only when it is reused, not "just in case". Keep native commands, React presentation and feature styles focused.
5. **Readability** — consistent naming (`snake_case` in Rust, `camelCase` in TypeScript, `kebab-case` in CSS), formatting that matches surrounding code, comments that explain *why* for non-obvious logic; delete comments that restate the code.
6. **Code smells** — duplicated code, overlong functions, oversized modules, over-design, unused items and dependencies.

## Rules

- Keep features, public interfaces, persisted formats (config JSON, recovery codes, log files) and user-visible behaviour unchanged. A refactor that changes behaviour is a bug.
- Never trade clarity for fewer lines.
- No new abstractions, design patterns, traits or dependencies unless they remove real duplication; never add a crate for a refactor.
- Write idiomatic Rust (clippy-clean where practical), React and TypeScript.
- Work in phases (one module or concern at a time). After each phase, in `native/`:
  - `cargo build --locked` with zero warnings;
  - `cargo test --locked` all green;
  - for UI changes also `npm run build` and `npm test` in `native/web/`, and actual Tauri screenshots of the touched pages under `skills/windows-desktop-validation/SKILL.md`;
  - `cargo test --locked --test all -- --ignored` covers the current opt-in integration checks; inspect which tests actually ran instead of treating that command as GUI evidence.
  Commit each phase separately with a message that states what was simplified.
- Builds: at most one cargo build at a time; Cargo is capped at half the CPUs in the user's Cargo config. Keep build output and caches off the C: drive.
- Every user-visible string goes through `i18n::t`/`tf!` (Rust) or `t` (TypeScript). Keep `src/i18n_catalog.rs` and the feature's `web/src/i18n/zh/` entries in sync with call sites. Apply `skills/desktop-ui-design/SKILL.md` to visual or motion changes.
- Tests may only change when the code they check was deliberately restructured; never weaken an assertion to make a refactor pass.

## Finish with a summary

List per phase: redundancy removed (with rough line counts), complexity reduced (functions split, nesting removed), dependencies or dead code dropped, and the issues that remain with the reason they were left.
