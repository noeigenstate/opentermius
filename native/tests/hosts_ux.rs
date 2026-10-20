//! Integration coverage for the hosts page data: persisted view preferences,
//! tags, timestamps and group management.
use opentermius::hosts::{self, HostsView, SortMode, ViewMode};
use opentermius::store::{Data, Profile, Store};

fn profile(id: &str, name: &str) -> Profile {
    Profile {
        id: id.into(),
        name: name.into(),
        host: format!("{id}.example.invalid"),
        port: 22,
        username: "tester".into(),
        auth_type: "password".into(),
        private_key_path: String::new(),
        key_id: String::new(),
        group: String::new(),
        encrypted_secret: None,
        tags: Vec::new(),
        last_connected: None,
        created_at: None,
    }
}

#[test]
fn legacy_config_gets_defaults_for_host_metadata() {
    let json = r#"{
        "version": 2,
        "hosts": [{"name": "Legacy", "host": "legacy.example.invalid", "username": "root"}]
    }"#;
    let data: Data = serde_json::from_str(json).unwrap();
    assert!(data.hosts[0].tags.is_empty());
    assert!(data.hosts[0].last_connected.is_none());
    assert!(data.hosts[0].created_at.is_none());
    assert_eq!(data.hosts_view, HostsView::default());
    assert_eq!(data.hosts_view.view_mode(), ViewMode::Grid);
    assert_eq!(data.hosts_view.sort_mode(), SortMode::Name);
}

#[test]
fn hosts_view_and_tags_round_trip_through_config() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("data.json");
    let mut store = Store::open(path.clone()).unwrap();
    store.data.hosts_view.mode = "list".into();
    store.data.hosts_view.sort = "created".into();
    let mut host = profile("a", "Alpha");
    host.tags = hosts::parse_tags("web, #Edge, web");
    store.save(host, None, false).unwrap();
    store.persist().unwrap();

    let reopened = Store::open(path).unwrap();
    assert_eq!(reopened.data.hosts_view.view_mode(), ViewMode::List);
    assert_eq!(reopened.data.hosts_view.sort_mode(), SortMode::Created);
    assert_eq!(reopened.data.hosts[0].tags, vec!["web", "Edge"]);
    assert!(
        reopened.data.hosts[0].created_at.is_some(),
        "new hosts get a created timestamp"
    );
}

#[test]
fn connection_time_survives_edits_and_reload() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("data.json");
    let mut store = Store::open(path.clone()).unwrap();
    let mut host = profile("a", "Alpha");
    host.tags = vec!["prod".into()];
    store.save(host, None, false).unwrap();
    let created = store.data.hosts[0].created_at.clone();
    assert!(hosts::mark_connected(&mut store, "a", "2026-01-02T03:04:05Z").unwrap());
    assert!(!hosts::mark_connected(&mut store, "missing", "2026-01-02T03:04:05Z").unwrap());

    let mut edited = store.data.hosts[0].clone();
    edited.name = "Alpha renamed".into();
    // The UI editor sends a fresh draft without historical metadata.
    edited.created_at = None;
    edited.last_connected = None;
    store.save(edited, None, false).unwrap();
    let reopened = Store::open(path).unwrap();
    assert_eq!(reopened.data.hosts[0].name, "Alpha renamed");
    assert_eq!(reopened.data.hosts[0].created_at, created);
    assert_eq!(
        reopened.data.hosts[0].last_connected.as_deref(),
        Some("2026-01-02T03:04:05Z")
    );
}

#[test]
fn duplicate_and_group_rename_are_persisted() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("data.json");
    let mut store = Store::open(path.clone()).unwrap();
    let mut first = profile("a", "Alpha");
    first.group = "dev".into();
    store.save(first, None, false).unwrap();
    let mut second = profile("b", "Beta");
    second.group = "dev".into();
    store.save(second, None, false).unwrap();

    let copy = hosts::duplicate_profile(&mut store, "a").unwrap();
    let copied = store
        .data
        .hosts
        .iter()
        .find(|host| host.id == copy)
        .unwrap();
    assert_eq!(copied.name, "Alpha (copy)");
    assert_eq!(copied.group, "dev");

    assert_eq!(hosts::rename_group(&mut store, "dev", "prod").unwrap(), 3);
    assert!(hosts::rename_group(&mut store, "prod", "").is_err());

    let reopened = Store::open(path).unwrap();
    assert_eq!(reopened.data.hosts.len(), 3);
    assert!(reopened.data.hosts.iter().all(|host| host.group == "prod"));
    assert!(reopened.data.hosts.iter().any(|host| host.id == copy));
}

#[test]
fn visible_hosts_are_filtered_and_sorted_together() {
    let mut alpha = profile("a", "Alpha");
    alpha.group = "prod".into();
    alpha.tags = vec!["web".into()];
    alpha.last_connected = Some("2026-01-01T00:00:00Z".into());
    let mut beta = profile("b", "Beta");
    beta.group = "prod".into();
    beta.tags = vec!["db".into()];
    beta.last_connected = Some("2026-02-01T00:00:00Z".into());
    let hosts = [alpha, beta];

    let filtered = hosts::visible_hosts(&hosts, "", "prod", "web", SortMode::Name);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, "a");

    let sorted = hosts::visible_hosts(&hosts, "", "", "", SortMode::LastConnected);
    assert_eq!(
        sorted
            .iter()
            .map(|host| host.id.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "a"]
    );
    assert!(hosts::visible_hosts(&hosts, "gamma", "", "", SortMode::Name).is_empty());

    let mut unicode = profile("c", "Unicode tag");
    unicode.tags = vec!["ÄPI".into()];
    assert!(hosts::has_tag(&unicode, "äpi"));
}

#[test]
fn autocomplete_matches_all_existing_groups_and_merges_counts() {
    let mut hosts: Vec<Profile> = (0..12)
        .map(|index| {
            let mut host = profile(&index.to_string(), "Host");
            host.group = format!("Production {index:02}");
            host
        })
        .collect();
    let mut extra = profile("extra", "Extra");
    extra.group = "Production 11".into();
    hosts.push(extra);
    assert_eq!(hosts::group_suggestions(&hosts, "production").len(), 12);
    assert_eq!(
        hosts::group_suggestions(&hosts, "  TION 11  "),
        vec![("Production 11".into(), 2)]
    );
    assert!(hosts::group_suggestions(&hosts, "missing").is_empty());
}

#[test]
fn failed_host_actions_restore_in_memory_data() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(temp.path().join("data.json")).unwrap();
    let mut host = profile("a", "Alpha");
    host.group = "dev".into();
    store.save(host, None, false).unwrap();
    let blocked = temp.path().join("blocked");
    std::fs::write(&blocked, "not a directory").unwrap();
    store.path = blocked.join("data.json");
    let before = serde_json::to_value(&store.data).unwrap();
    assert!(hosts::mark_connected(&mut store, "a", "2026-01-01T00:00:00Z").is_err());
    assert_eq!(serde_json::to_value(&store.data).unwrap(), before);
    assert!(hosts::duplicate_profile(&mut store, "a").is_err());
    assert_eq!(serde_json::to_value(&store.data).unwrap(), before);
    assert!(hosts::rename_group(&mut store, "dev", "prod").is_err());
    assert_eq!(serde_json::to_value(&store.data).unwrap(), before);
    assert!(hosts::mark_connected(&mut store, "a", "invalid").is_err());
}

#[test]
fn date_sort_compares_instants_across_timezones_and_missing_dates() {
    let mut a = profile("a", "Alpha");
    a.last_connected = Some("2026-01-01T09:00:00+08:00".into());
    let mut b = profile("b", "Beta");
    b.last_connected = Some("2026-01-01T02:00:00Z".into());
    let c = profile("c", "Never connected");
    let hosts = [a, b, c];
    let sorted = hosts::visible_hosts(&hosts, "", "", "", SortMode::LastConnected);
    assert_eq!(
        sorted.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        ["b", "a", "c"]
    );
}

#[test]
fn duplicate_names_remain_unique_beyond_one_thousand_copies() {
    let mut names = vec!["Prod (copy)".to_string()];
    names.extend((2..=1000).map(|index| format!("Prod (copy {index})")));
    assert_eq!(
        hosts::duplicate_name("Prod", names.iter().map(String::as_str)),
        "Prod (copy 1001)"
    );
}

#[test]
fn duplicating_maximum_length_names_keeps_config_readable() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(temp.path().join("data.json")).unwrap();
    for (id, name) in [("ascii", "x".repeat(128)), ("unicode", "🖥".repeat(128))] {
        store.save(profile(id, &name), None, false).unwrap();
        let copied_id = hosts::duplicate_profile(&mut store, id).unwrap();
        let copy = store.data.hosts.iter().find(|p| p.id == copied_id).unwrap();
        assert!(copy.name.len() <= 512);
        assert!(copy.name.chars().count() <= 128);
        assert!(copy.name.ends_with(" (copy)"));
    }
    assert_eq!(Store::open(store.path.clone()).unwrap().data.hosts.len(), 4);
}

#[cfg(windows)]
#[test]
fn duplicate_keeps_encrypted_credentials_but_resets_connection_history() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(temp.path().join("data.json")).unwrap();
    let mut host = profile("a", "Alpha");
    host.last_connected = Some("2026-01-01T00:00:00Z".into());
    host.tags = vec!["prod".into()];
    store.save(host, Some("fixture-secret"), true).unwrap();
    let id = hosts::duplicate_profile(&mut store, "a").unwrap();
    let copy = store.data.hosts.iter().find(|p| p.id == id).unwrap();
    assert_eq!(copy.encrypted_secret, store.data.hosts[0].encrypted_secret);
    assert_eq!(copy.tags, ["prod"]);
    assert!(copy.last_connected.is_none());
    assert_eq!(
        opentermius::store::unprotect(copy.encrypted_secret.as_deref().unwrap()).unwrap(),
        "fixture-secret"
    );
    assert!(
        !std::fs::read_to_string(&store.path)
            .unwrap()
            .contains("fixture-secret")
    );
}
