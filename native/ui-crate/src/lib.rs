//! Compiled UI shared by the application and its page controllers.
//!
//! The generated module publicly exports AppWindow, bridge globals, and row
//! types. Keeping it here avoids compiling them again after a Rust app edit.
slint::include_modules!();
