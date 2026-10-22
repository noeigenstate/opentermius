//! One integration-test executable. Keep fixtures scoped to their original
//! modules so test names remain selectable as `<module>::<test>`.
mod hosts_ux;
mod import_windterm;
mod import_xshell;
mod integration_features;
mod integration_recovery_transport;
mod keychain;
mod local_terminal;
mod quick_connect;
mod recovery;
mod security;
mod settings;
mod sftp_ops;
mod snippets;
mod store_core;
mod terminal_core;
mod transport;
mod update;
