//! CPU, memory and root-disk usage of a connected Linux server, read through a short exec
//! channel on the existing SSH connection. Only `/proc` and `df` are read; nothing is
//! installed or changed on the server. Other systems simply report no values.

/// POSIX shell snippet; `LC_ALL=C` keeps `df` output parseable.
pub const COMMAND: &str = "export LC_ALL=C; head -n 1 /proc/stat; grep -E '^(MemTotal|MemAvailable):' /proc/meminfo; df -P / | tail -n 1";

/// Percentages 0–100; `None` when unknown (first CPU sample, or not a Linux server).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub cpu: Option<u8>,
    pub memory: Option<u8>,
    pub disk: Option<u8>,
}

impl Stats {
    pub fn is_empty(&self) -> bool {
        self.cpu.is_none() && self.memory.is_none() && self.disk.is_none()
    }
}

/// Cumulative CPU counters from `/proc/stat`; usage is the difference between two samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuSample {
    pub busy: u64,
    pub total: u64,
}

fn percent(part: u64, whole: u64) -> Option<u8> {
    (whole > 0).then(|| (part.min(whole) * 100 / whole) as u8)
}

/// Parses the output of [`COMMAND`]; returns the stats and the CPU sample for next time.
pub fn parse(output: &str, previous: Option<CpuSample>) -> (Stats, Option<CpuSample>) {
    let mut stats = Stats::default();
    let mut sample = None;
    let (mut mem_total, mut mem_available) = (None, None);
    for line in output.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields.first().copied() {
            Some("cpu") => {
                // user nice system idle iowait irq softirq steal
                let values: Vec<u64> = fields[1..].iter().take(8).filter_map(|v| v.parse().ok()).collect();
                if values.len() >= 4 {
                    let total: u64 = values.iter().sum();
                    let idle = values[3] + values.get(4).copied().unwrap_or(0);
                    let current = CpuSample { busy: total - idle, total };
                    if let Some(prev) = previous.filter(|p| current.total > p.total && current.busy >= p.busy) {
                        stats.cpu = percent(current.busy - prev.busy, current.total - prev.total);
                    }
                    sample = Some(current);
                }
            }
            Some("MemTotal:") => mem_total = fields.get(1).and_then(|v| v.parse::<u64>().ok()),
            Some("MemAvailable:") => mem_available = fields.get(1).and_then(|v| v.parse::<u64>().ok()),
            _ => {
                // `df -P`: filesystem blocks used available capacity mountpoint
                if fields.len() >= 6 && fields[5] == "/" {
                    if let Some(capacity) = fields[4].strip_suffix('%').and_then(|v| v.parse::<u8>().ok()) {
                        stats.disk = Some(capacity.min(100));
                    }
                }
            }
        }
    }
    if let (Some(total), Some(available)) = (mem_total, mem_available) {
        stats.memory = percent(total.saturating_sub(available), total);
    }
    (stats, sample)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: &str = "cpu  100 0 100 800 0 0 0 0 0 0\nMemTotal:       2000000 kB\nMemAvailable:   1500000 kB\n/dev/vda1         20000000  9800000  10200000      49% /\n";
    const SECOND: &str = "cpu  150 0 150 850 0 0 0 0 0 0\nMemTotal:       2000000 kB\nMemAvailable:   1400000 kB\n/dev/vda1         20000000  9800000  10200000      49% /\n";

    #[test]
    fn cpu_needs_two_samples_memory_and_disk_do_not() {
        let (first, sample) = parse(FIRST, None);
        assert_eq!(first, Stats { cpu: None, memory: Some(25), disk: Some(49) });
        let (second, _) = parse(SECOND, sample);
        // 100 busy of 150 total ticks between the samples.
        assert_eq!(second, Stats { cpu: Some(66), memory: Some(30), disk: Some(49) });
    }

    #[test]
    fn unknown_or_garbage_output_reports_nothing() {
        assert!(parse("", None).0.is_empty());
        assert!(parse("sh: /proc/stat: No such file or directory\nFilesystem 512-blocks Used Available Capacity Mounted on\n", None).0.is_empty());
        let (stats, _) = parse("cpu 1 2\nMemTotal: 0 kB\nMemAvailable: 0 kB\n", None);
        assert!(stats.is_empty());
        // A counter reset (reboot) must not produce a bogus value.
        let (stats, _) = parse(FIRST, Some(CpuSample { busy: 999_999, total: 9_999_999 }));
        assert_eq!(stats.cpu, None);
    }
}
