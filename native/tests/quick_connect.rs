use opentermius::store::parse_quick_connect;
use std::collections::HashSet;

fn parsed(value: &str) -> opentermius::store::Profile {
    parse_quick_connect(value).unwrap_or_else(|error| panic!("{value:?} should parse: {error:#}"))
}

fn rejected(value: &str) {
    assert!(
        parse_quick_connect(value).is_err(),
        "{value:?} should be rejected"
    );
}

#[test]
fn accepts_user_at_host_and_documented_ssh_form() {
    let p = parsed("tester@example.invalid");
    assert_eq!(
        (p.username.as_str(), p.host.as_str(), p.port),
        ("tester", "example.invalid", 22)
    );
    assert_eq!(p.name, "example.invalid");
    assert_eq!(p.auth_type, "password");
    assert!(!p.id.is_empty());

    let p = parsed("ssh tester@example.invalid -p 2222");
    assert_eq!(
        (p.username.as_str(), p.host.as_str(), p.port),
        ("tester", "example.invalid", 2222)
    );

    let p = parsed("  ssh   tester@example.invalid   -p   2222  ");
    assert_eq!(
        (p.username.as_str(), p.host.as_str(), p.port),
        ("tester", "example.invalid", 2222)
    );
}

#[test]
fn accepts_login_name_option_before_or_after_host() {
    let p = parsed("ssh -l alice example.invalid");
    assert_eq!(
        (p.username.as_str(), p.host.as_str()),
        ("alice", "example.invalid")
    );

    let p = parsed("example.invalid -l alice");
    assert_eq!(
        (p.username.as_str(), p.host.as_str()),
        ("alice", "example.invalid")
    );

    // OpenSSH gives the user in the destination precedence over -l.
    let p = parsed("ssh -l alice bob@example.invalid");
    assert_eq!(
        (p.username.as_str(), p.host.as_str()),
        ("bob", "example.invalid")
    );
}

#[test]
fn parses_host_colon_port_and_bracketed_ipv6() {
    let p = parsed("example.invalid:2222");
    assert_eq!((p.host.as_str(), p.port), ("example.invalid", 2222));

    let p = parsed("tester@example.invalid:2222");
    assert_eq!(
        (p.username.as_str(), p.host.as_str(), p.port),
        ("tester", "example.invalid", 2222)
    );

    let p = parsed("[::1]");
    assert_eq!((p.host.as_str(), p.port), ("::1", 22));

    let p = parsed("tester@[::1]");
    assert_eq!(
        (p.username.as_str(), p.host.as_str(), p.port),
        ("tester", "::1", 22)
    );

    let p = parsed("[::1]:2222");
    assert_eq!((p.host.as_str(), p.port), ("::1", 2222));

    let p = parsed("tester@[2001:db8::1]:2200");
    assert_eq!(
        (p.username.as_str(), p.host.as_str(), p.port),
        ("tester", "2001:db8::1", 2200)
    );

    let p = parsed("[fe80::1%25eth0]:22");
    assert_eq!((p.host.as_str(), p.port), ("fe80::1%25eth0", 22));

    let p = parsed("2001:db8::1");
    assert_eq!((p.host.as_str(), p.port), ("2001:db8::1", 22));

    let p = parsed("::1");
    assert_eq!((p.host.as_str(), p.port), ("::1", 22));
}

#[test]
fn explicit_port_option_wins_over_host_suffix() {
    let p = parsed("ssh example.invalid:2222 -p 2200");
    assert_eq!(p.port, 2200);
    let p = parsed("ssh -p 2200 example.invalid:2222");
    assert_eq!(p.port, 2200);
}

#[test]
fn rejects_invalid_hosts_ports_and_brackets() {
    for value in [
        "",
        "   ",
        "ssh",
        "ssh -p 22",
        "ssh example.invalid:0",
        "ssh example.invalid:65536",
        "ssh example.invalid:",
        "ssh [::1",
        "ssh ::1]",
        "ssh []",
        "ssh [::1]extra",
        "ssh [::1]:22:33",
        "ssh [example.invalid]",
        "ssh -p 0 example.invalid",
        "ssh -p 65536 example.invalid",
        "ssh -p nope example.invalid",
        "ssh -p",
        "ssh -l",
        "ssh -i",
        "ssh -i '' example.invalid",
        "ssh example.invalid extra",
        "ssh example.invalid rm -rf /",
        "ssh 'unclosed",
    ] {
        rejected(value);
    }
}

#[test]
fn host_name_must_not_contain_whitespace_or_control_characters() {
    rejected("ssh 'bad host'");
    rejected("ssh 'bad\thost'");
    rejected("ssh 'bad\nhost'");
    rejected("ssh \"bad host:22\"");
    rejected("ssh '例子 .测试'");
}

#[test]
fn injection_looking_input_is_data_or_an_error_never_executed() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("quick-connect-pwned");
    let marker_text = marker.display().to_string();

    // One token containing whitespace, so it is rejected as an invalid host.
    rejected(&format!("ssh 'example.invalid; touch {marker_text}'"));
    // Separate shell tokens are rejected because a remote command is not supported.
    rejected(&format!("ssh example.invalid; touch {marker_text}"));
    rejected(&format!("ssh example.invalid && touch {marker_text}"));
    rejected(&format!("ssh $(touch {marker_text})"));
    rejected(&format!("ssh `touch {marker_text}`"));
    rejected(&format!("ssh example.invalid | touch {marker_text}"));

    // No whitespace: the text is kept as literal host data and never run.
    let p = parsed("ssh 'example.invalid;touch'");
    assert_eq!(p.host, "example.invalid;touch");
    assert!(!marker.exists(), "quick connect must not execute input");

    // Quoted values keep metacharacters as data instead of treating them as syntax.
    let p = parsed("ssh 'user$name@example.invalid'");
    assert_eq!(p.username, "user$name");
    assert_eq!(p.host, "example.invalid");
}

#[test]
fn quoted_arguments_and_unicode_hosts_are_supported() {
    let p = parsed("ssh \"tester@example.invalid\" -p '2222'");
    assert_eq!(
        (p.username.as_str(), p.host.as_str(), p.port),
        ("tester", "example.invalid", 2222)
    );

    let p = parsed("ssh '例子.测试'");
    assert_eq!(p.host, "例子.测试");
    assert_eq!(p.name, "例子.测试");

    let p = parsed("ssh '例子.测试:2222'");
    assert_eq!((p.host.as_str(), p.port), ("例子.测试", 2222));
}

#[test]
fn long_valid_hostnames_do_not_exceed_the_display_name_limit() {
    let host = format!(
        "{}.{}.{}.{}",
        "a".repeat(63),
        "b".repeat(63),
        "c".repeat(63),
        "d".repeat(61)
    );
    let mut p = parsed(&format!("tester@{host}"));
    assert_eq!(p.host, host);
    assert!(p.name.chars().count() <= 128);
    p.validate().unwrap();
}

#[test]
fn key_authentication_option_sets_type_and_preserves_windows_paths() {
    let p = parsed("ssh -i 'C:\\Users\\me\\.ssh\\id_ed25519' tester@example.invalid");
    assert_eq!(p.auth_type, "key");
    assert_eq!(p.private_key_path, "C:\\Users\\me\\.ssh\\id_ed25519");

    let p = parsed("ssh -i '/home/me/key' tester@example.invalid");
    assert_eq!(p.auth_type, "key");
    assert_eq!(p.private_key_path, "/home/me/key");

    let p = parsed("ssh tester@example.invalid");
    assert_eq!(p.auth_type, "password");
    assert!(p.private_key_path.is_empty());
}

#[test]
fn empty_destination_user_is_left_for_the_credential_prompt() {
    let p = parsed("ssh @example.invalid");
    assert_eq!(p.username, "");
    assert_eq!(p.host, "example.invalid");
}

#[test]
fn generated_ids_are_unique_and_not_reused() {
    let ids: HashSet<_> = (0..32)
        .map(|_| parsed("tester@example.invalid").id)
        .collect();
    assert_eq!(ids.len(), 32);
    assert!(ids.iter().all(|id| !id.is_empty()));
}

#[test]
fn rejects_unquoted_control_characters_and_nul() {
    rejected("ssh bad\u{0}host");
    rejected("ssh bad\u{1}host");
    rejected("ssh bad\rhost");
}
