//! Host list presentation: tags, filtering, sorting and host actions.
//!
//! The functions here are deliberately independent from the UI so the host
//! page logic can be unit tested. They operate on [`Profile`] and [`Store`].
use crate::store::{Profile, Store};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

pub const MAX_TAGS: usize = 16;
pub const MAX_TAG_CHARS: usize = 32;

/// The two ways the hosts page can render saved hosts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Grid,
    List,
}

impl ViewMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::List => "list",
        }
    }
    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("list") {
            Self::List
        } else {
            Self::Grid
        }
    }
}

/// Sort orders offered by the hosts toolbar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortMode {
    #[default]
    Name,
    LastConnected,
    Created,
}

impl SortMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::LastConnected => "last",
            Self::Created => "created",
        }
    }
    pub fn parse(value: &str) -> Self {
        if value.eq_ignore_ascii_case("last") || value.eq_ignore_ascii_case("lastConnected") {
            Self::LastConnected
        } else if value.eq_ignore_ascii_case("created") {
            Self::Created
        } else {
            Self::Name
        }
    }
}

/// Persisted hosts page preferences. Stored in `store::Data::hosts_view`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HostsView {
    pub mode: String,
    pub sort: String,
}

impl Default for HostsView {
    fn default() -> Self {
        Self {
            mode: ViewMode::Grid.as_str().into(),
            sort: SortMode::Name.as_str().into(),
        }
    }
}

impl HostsView {
    pub fn view_mode(&self) -> ViewMode {
        ViewMode::parse(&self.mode)
    }
    pub fn sort_mode(&self) -> SortMode {
        SortMode::parse(&self.sort)
    }
    pub fn set_mode(&mut self, mode: ViewMode) {
        self.mode = mode.as_str().into();
    }
    pub fn set_sort(&mut self, sort: SortMode) {
        self.sort = sort.as_str().into();
    }
    /// Replaces unknown values with their defaults.
    pub fn normalized(mut self) -> Self {
        self.set_mode(self.view_mode());
        self.set_sort(self.sort_mode());
        self
    }
}

/// Splits the editor's comma separated tag field, dropping duplicates and
/// control characters. A leading `#` is accepted for convenience.
pub fn parse_tags(input: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for raw in input.split(',') {
        let tag = raw.trim().trim_start_matches('#').trim();
        if tag.is_empty() || tag.chars().any(char::is_control) {
            continue;
        }
        let tag: String = tag.chars().take(MAX_TAG_CHARS).collect();
        if tags
            .iter()
            .any(|known| known.to_lowercase() == tag.to_lowercase())
        {
            continue;
        }
        tags.push(tag);
        if tags.len() == MAX_TAGS {
            break;
        }
    }
    tags
}

pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    parse_tags(&tags.join(","))
}

pub fn tags_string(profile: &Profile) -> String {
    profile.tags.join(", ")
}

pub fn has_tag(profile: &Profile, tag: &str) -> bool {
    tag.is_empty()
        || profile
            .tags
            .iter()
            .any(|candidate| candidate.to_lowercase() == tag.to_lowercase())
}

/// Free-text search across the fields shown for a host.
pub fn matches_search(profile: &Profile, query_lower: &str) -> bool {
    query_lower.is_empty()
        || profile.name.to_lowercase().contains(query_lower)
        || profile.host.to_lowercase().contains(query_lower)
        || profile.username.to_lowercase().contains(query_lower)
        || profile.group.to_lowercase().contains(query_lower)
        || profile
            .tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(query_lower))
}

/// Hosts visible for the current search, group and tag filters, sorted by the
/// chosen order. The returned references preserve the original profile data.
/// Group filter value selecting hosts that are not in any group.
pub const NO_GROUP: &str = "\u{1}";

pub fn visible_hosts<'a>(
    hosts: &'a [Profile],
    search: &str,
    group: &str,
    tag: &str,
    sort: SortMode,
) -> Vec<&'a Profile> {
    let query = search.trim().trim_start_matches('#').to_lowercase();
    let mut visible: Vec<&Profile> = hosts
        .iter()
        .filter(|profile| group.is_empty() || profile.group == group || (group == NO_GROUP && profile.group.is_empty()))
        .filter(|profile| has_tag(profile, tag))
        .filter(|profile| matches_search(profile, &query))
        .collect();
    sort_hosts(&mut visible, sort);
    visible
}

fn timestamp(value: Option<&str>) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    value.and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
}

fn compare_name(a: &Profile, b: &Profile) -> std::cmp::Ordering {
    a.name
        .to_lowercase()
        .cmp(&b.name.to_lowercase())
        .then_with(|| a.name.cmp(&b.name))
        .then_with(|| a.id.cmp(&b.id))
}

pub fn sort_hosts(hosts: &mut [&Profile], sort: SortMode) {
    hosts.sort_by(|a, b| {
        let order = match sort {
            SortMode::Name => compare_name(a, b),
            SortMode::LastConnected => timestamp(b.last_connected.as_deref())
                .cmp(&timestamp(a.last_connected.as_deref()))
                .then_with(|| compare_name(a, b)),
            SortMode::Created => timestamp(b.created_at.as_deref())
                .cmp(&timestamp(a.created_at.as_deref()))
                .then_with(|| compare_name(a, b)),
        };
        order
    });
}

/// Distinct tags with the number of hosts carrying each one, sorted by name.
pub fn tag_counts(hosts: &[Profile]) -> Vec<(String, i32)> {
    let mut counts = BTreeMap::<String, (String, i32)>::new();
    for profile in hosts {
        for tag in &profile.tags {
            let entry = counts
                .entry(tag.to_lowercase())
                .or_insert_with(|| (tag.clone(), 0));
            entry.1 += 1;
        }
    }
    counts.into_values().collect()
}

/// Generates a unique "Name (copy)" label for duplicated hosts.
pub fn duplicate_name<'a>(base: &str, existing: impl IntoIterator<Item = &'a str>) -> String {
    fn label(base: &str, suffix: &str) -> String {
        // Leave room for the suffix within Profile's character and byte limits.
        let mut name = String::new();
        for ch in base.chars().take(128 - suffix.chars().count()) {
            if name.len() + ch.len_utf8() + suffix.len() > 512 {
                break;
            }
            name.push(ch);
        }
        name.push_str(suffix);
        name
    }
    let existing: Vec<String> = existing.into_iter().map(str::to_lowercase).collect();
    let taken = |candidate: &str| existing.iter().any(|name| name == candidate);
    let base = base.trim();
    let base = if base.is_empty() { crate::i18n::t("Host") } else { base };
    let first = label(base, crate::i18n::t(" (copy)"));
    if !taken(&first.to_lowercase()) {
        return first;
    }
    for number in 2..=existing.len() + 2 {
        let candidate = label(base, &crate::tf!(" (copy {number})", "number" => format!("{}", number)));
        if !taken(&candidate.to_lowercase()) {
            return candidate;
        }
    }
    unreachable!("a finite set of names cannot exhaust all copy suffixes")
}

/// Existing groups matching the editor input, with their host counts.
pub fn group_suggestions(hosts: &[Profile], query: &str) -> Vec<(String, i32)> {
    let query = query.trim().to_lowercase();
    let mut groups = BTreeMap::<String, i32>::new();
    for profile in hosts {
        if !profile.group.is_empty() && profile.group.to_lowercase().contains(&query) {
            *groups.entry(profile.group.clone()).or_default() += 1;
        }
    }
    groups.into_iter().collect()
}

/// The address shown by "Copy address".
pub fn copy_address(profile: &Profile) -> String {
    let host = if profile.host.contains(':') && !profile.host.starts_with('[') {
        format!("[{}]", profile.host)
    } else {
        profile.host.clone()
    };
    if profile.username.is_empty() {
        format!("{host} -p {}", profile.port)
    } else {
        format!("{}@{host} -p {}", profile.username, profile.port)
    }
}

/// Moves a selection through a list of visible host ids without wrapping.
pub fn next_selection(ids: &[String], current: &str, delta: i32) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let last = ids.len() - 1;
    let index = match ids.iter().position(|id| id == current) {
        Some(index) => (index as i32 + delta).clamp(0, last as i32) as usize,
        None if delta < 0 => last,
        None => 0,
    };
    ids.get(index).cloned()
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Records a successful connection. Returns `false` when the profile is not
/// stored (for example a quick-connect session).
pub fn mark_connected(store: &mut Store, id: &str, now: &str) -> Result<bool> {
    ensure!(chrono::DateTime::parse_from_rfc3339(now).is_ok(), "{}", crate::i18n::t("Invalid connection timestamp"));
    let Some(index) = store.data.hosts.iter().position(|profile| profile.id == id) else {
        return Ok(false);
    };
    let previous = store.data.hosts[index].last_connected.clone();
    store.data.hosts[index].last_connected = Some(now.to_string());
    if let Err(error) = store.persist() {
        store.data.hosts[index].last_connected = previous;
        return Err(error);
    }
    Ok(true)
}

/// Sets the saved favorite flag, restoring it if persistence fails.
pub fn set_favorite(store: &mut Store, id: &str, favorite: bool) -> Result<()> {
    let index = store.data.hosts.iter().position(|profile| profile.id == id)
        .context(crate::i18n::t("Host not found"))?;
    let previous = store.data.hosts[index].favorite;
    store.data.hosts[index].favorite = favorite;
    if let Err(error) = store.persist() {
        store.data.hosts[index].favorite = previous;
        return Err(error);
    }
    Ok(())
}

/// Duplicates a host with a unique "(copy)" name. Saved credentials stay in
/// their encrypted form and are copied as-is.
pub fn duplicate_profile(store: &mut Store, id: &str) -> Result<String> {
    let source = store
        .data
        .hosts
        .iter()
        .find(|profile| profile.id == id)
        .cloned()
        .context(crate::i18n::t("Host not found"))?;
    let new_id = Uuid::new_v4().to_string();
    let name = duplicate_name(
        &source.name,
        store.data.hosts.iter().map(|profile| profile.name.as_str()),
    );
    let mut copy = source;
    copy.id = new_id.clone();
    copy.name = name;
    copy.created_at = Some(now_rfc3339());
    copy.last_connected = None;
    copy.favorite = false;
    copy.validate()?;
    store.data.hosts.push(copy);
    if let Err(error) = store.persist() {
        store.data.hosts.pop();
        return Err(error);
    }
    Ok(new_id)
}

/// Renames a group and every host that belongs to it, merging into an existing
/// group when the target name is already in use.
pub fn rename_group(store: &mut Store, old: &str, new: &str) -> Result<usize> {
    let new = new.trim();
    ensure!(!new.is_empty(), "{}", crate::i18n::t("Group name cannot be empty"));
    ensure!(new.chars().count() <= 128 && !new.chars().any(char::is_control), "{}", crate::i18n::t("Invalid group name"));
    if old == new {
        return Ok(0);
    }
    let previous = store.data.clone();
    let mut changed = 0;
    for profile in &mut store.data.hosts {
        if profile.group == old {
            profile.group = new.to_string();
            changed += 1;
        }
    }
    if changed == 0 {
        return Ok(0);
    }
    if let Err(error) = store.persist() {
        store.data = previous;
        return Err(error);
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

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
            tags: Vec::new(),
            last_connected: None,
            favorite: false,
            created_at: None,
            encrypted_secret: None,
        }
    }

    #[test]
    fn tags_are_trimmed_deduplicated_and_capped() {
        assert_eq!(
            parse_tags(" dev, #Prod ,dev,, ops "),
            vec!["dev".to_string(), "Prod".into(), "ops".into()]
        );
        let many: Vec<String> = (0..40).map(|index| format!("tag{index}")).collect();
        assert_eq!(parse_tags(&many.join(",")).len(), MAX_TAGS);
        let long = "x".repeat(80);
        assert_eq!(
            parse_tags(&long)[0].chars().count(),
            MAX_TAG_CHARS,
            "tag length is capped"
        );
        assert!(parse_tags(" , # , ").is_empty());
        assert_eq!(parse_tags("ÄPI, äpi"), ["ÄPI"]);
    }

    #[test]
    fn filtering_combines_group_tag_and_search() {
        let mut a = profile("a", "Alpha");
        a.group = "prod".into();
        a.tags = vec!["web".into(), "edge".into()];
        let mut b = profile("b", "Beta");
        b.group = "dev".into();
        b.tags = vec!["db".into()];
        let hosts = vec![a, b];
        assert_eq!(
            visible_hosts(&hosts, "", "prod", "", SortMode::Name).len(),
            1
        );
        assert_eq!(
            visible_hosts(&hosts, "", "", "WEB", SortMode::Name).len(),
            1
        );
        assert_eq!(
            visible_hosts(&hosts, "beta", "", "", SortMode::Name).len(),
            1
        );
        assert_eq!(
            visible_hosts(&hosts, "#edge", "", "", SortMode::Name).len(),
            1
        );
        assert_eq!(
            visible_hosts(&hosts, "", "", "missing", SortMode::Name).len(),
            0
        );
    }

    #[test]
    fn sorting_is_deterministic() {
        let mut a = profile("a", "zeta");
        a.created_at = Some("2024-01-01T00:00:00Z".into());
        a.last_connected = Some("2024-02-01T00:00:00Z".into());
        let mut b = profile("b", "Alpha");
        b.created_at = Some("2024-03-01T00:00:00Z".into());
        b.last_connected = Some("2024-04-01T00:00:00Z".into());
        let c = profile("c", "middle");
        let mut hosts = vec![&a, &b, &c];
        sort_hosts(&mut hosts, SortMode::Name);
        assert_eq!(
            hosts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["Alpha", "middle", "zeta"]
        );
        sort_hosts(&mut hosts, SortMode::LastConnected);
        assert_eq!(
            hosts.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["b", "a", "c"]
        );
        sort_hosts(&mut hosts, SortMode::Created);
        assert_eq!(
            hosts.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            vec!["b", "a", "c"]
        );
    }

    #[test]
    fn duplicate_names_increment() {
        let existing = ["Prod", "Prod (copy)"];
        assert_eq!(duplicate_name("Prod", existing), "Prod (copy 2)");
        assert_eq!(duplicate_name("New", existing), "New (copy)");
        assert_eq!(duplicate_name("", existing), "Host (copy)");
        assert_eq!(duplicate_name("prod", ["PROD (COPY)"]), "prod (copy 2)");
    }

    #[test]
    fn copy_address_handles_ports_and_ipv6() {
        let mut p = profile("a", "A");
        p.username = "root".into();
        p.port = 2222;
        assert_eq!(copy_address(&p), "root@a.example.invalid -p 2222");
        p.username.clear();
        p.host = "::1".into();
        assert_eq!(copy_address(&p), "[::1] -p 2222");
    }

    #[test]
    fn selection_clamps_at_the_edges() {
        let ids = vec!["a".to_string(), "b".into(), "c".into()];
        assert_eq!(next_selection(&ids, "b", 1).as_deref(), Some("c"));
        assert_eq!(next_selection(&ids, "c", 1).as_deref(), Some("c"));
        assert_eq!(next_selection(&ids, "a", -1).as_deref(), Some("a"));
        assert_eq!(next_selection(&ids, "", 1).as_deref(), Some("a"));
        assert_eq!(next_selection(&ids, "", -1).as_deref(), Some("c"));
        assert_eq!(next_selection(&[], "a", 1), None);
    }

    #[test]
    fn view_settings_are_normalized() {
        let view = HostsView {
            mode: "LIST".into(),
            sort: "created".into(),
        };
        assert_eq!(view.view_mode(), ViewMode::List);
        assert_eq!(view.sort_mode(), SortMode::Created);
        let view = HostsView {
            mode: "unknown".into(),
            sort: "whatever".into(),
        }
        .normalized();
        assert_eq!((view.mode.as_str(), view.sort.as_str()), ("grid", "name"));
        assert_eq!(HostsView::default().view_mode(), ViewMode::Grid);
    }

    #[test]
    fn store_actions_persist() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = store::Store::open(temp.path().join("data.json")).unwrap();
        let mut p = profile("a", "Alpha");
        p.tags = vec!["web".into()];
        p.created_at = Some("2024-01-01T00:00:00Z".into());
        store.save(p, None, false).unwrap();
        assert!(mark_connected(&mut store, "a", "2024-05-01T00:00:00Z").unwrap());
        assert!(!mark_connected(&mut store, "missing", "2024-05-01T00:00:00Z").unwrap());
        let copy = duplicate_profile(&mut store, "a").unwrap();
        assert_eq!(
            store.data.hosts.iter().find(|p| p.id == copy).unwrap().name,
            "Alpha (copy)"
        );
        assert_eq!(rename_group(&mut store, "", "prod").unwrap(), 2);
        assert_eq!(rename_group(&mut store, "prod", "prod").unwrap(), 0);
        assert!(rename_group(&mut store, "prod", " ").is_err());
        let reopened = store::Store::open(store.path.clone()).unwrap();
        assert_eq!(reopened.data.hosts.len(), 2);
        assert!(reopened.data.hosts.iter().all(|p| p.group == "prod"));
        assert_eq!(
            reopened
                .data
                .hosts
                .iter()
                .find(|p| p.id == "a")
                .unwrap()
                .last_connected
                .as_deref(),
            Some("2024-05-01T00:00:00Z")
        );
    }
}
