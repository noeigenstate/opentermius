//! Read-only OS discovery output; no remote text is persisted verbatim.
pub const COMMAND: &str = "cat /etc/os-release 2>/dev/null; uname -s 2>/dev/null";
pub const WINDOWS_COMMAND: &str = "cmd /c ver";

pub fn normalize_id(id: &str) -> Option<&'static str> {
    match id {
        "ubuntu" => Some("ubuntu"),
        "debian" => Some("debian"),
        "centos" => Some("centos"),
        "rhel" => Some("rhel"),
        "fedora" => Some("fedora"),
        "rocky" => Some("rocky"),
        "almalinux" => Some("almalinux"),
        "alpine" => Some("alpine"),
        "arch" => Some("arch"),
        "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "sles" => Some("opensuse"),
        "amzn" | "amazon" => Some("amazon"),
        "kali" => Some("kali"),
        "raspbian" => Some("raspbian"),
        "linux" => Some("linux"),
        "freebsd" => Some("freebsd"),
        "macos" => Some("macos"),
        "windows" => Some("windows"),
        _ => None,
    }
}

/// Prefer the distribution ID, then its closest known parent, then the kernel.
pub fn parse(output: &str) -> Option<&'static str> {
    let mut id = "";
    let mut like = "";
    for line in output.lines() {
        if let Some((key, value)) = line.trim().split_once('=') {
            let value = value.trim().trim_matches(['\'', '"']);
            match key {
                "ID" => id = value,
                "ID_LIKE" => like = value,
                _ => {}
            }
        }
    }
    normalize_id(id)
        .or_else(|| like.split_whitespace().find_map(normalize_id))
        .or_else(|| {
            output.lines().find_map(|line| match line.trim() {
                "Linux" => Some("linux"),
                "FreeBSD" => Some("freebsd"),
                "Darwin" => Some("macos"),
                "Windows_NT" => Some("windows"),
                line if line.starts_with("Microsoft Windows [") => Some("windows"),
                _ => None,
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distributions_prefer_id_over_parent_and_kernel() {
        for (id, expected) in [
            ("ubuntu", "ubuntu"),
            ("debian", "debian"),
            ("centos", "centos"),
            ("rhel", "rhel"),
            ("fedora", "fedora"),
            ("rocky", "rocky"),
            ("almalinux", "almalinux"),
            ("alpine", "alpine"),
            ("arch", "arch"),
            ("opensuse-leap", "opensuse"),
            ("opensuse-tumbleweed", "opensuse"),
            ("sles", "opensuse"),
            ("amzn", "amazon"),
            ("kali", "kali"),
            ("raspbian", "raspbian"),
        ] {
            let output = format!("NAME=Fixture\nID=\"{id}\"\nID_LIKE=\"debian rhel\"\nLinux\n");
            assert_eq!(parse(&output), Some(expected), "{id}");
        }
    }

    #[test]
    fn parents_quotes_crlf_and_generic_kernels() {
        for (output, expected) in [
            (
                "ID=derivative\nID_LIKE='unknown ubuntu debian'\nLinux",
                Some("ubuntu"),
            ),
            (
                "ID_LIKE=\"arch\"\r\nID=derivative\r\nLinux\r\n",
                Some("arch"),
            ),
            (
                "ID=derivative\nID_LIKE=\"suse opensuse\"\nLinux",
                Some("opensuse"),
            ),
            ("ID=derivative\nLinux", Some("linux")),
            ("Linux\n", Some("linux")),
            ("FreeBSD\n", Some("freebsd")),
            ("Darwin\n", Some("macos")),
            (
                "\r\nMicrosoft Windows [Version 10.0.20348.0]\r\n",
                Some("windows"),
            ),
            ("Windows_NT\r\n", Some("windows")),
            ("", None),
            ("ID=unknown\nPlan9", None),
            ("uname: command not found", None),
            ("NAME=Windows\nPRETTY_NAME=Ubuntu", None),
        ] {
            assert_eq!(parse(output), expected, "{output}");
        }
    }
}
