//! Native CLI version floors. Integrations name the oldest build they were verified on and
//! accept every newer one: provider CLIs ship often, and an exact pin breaks every person on
//! each release. Readers of native formats still fail closed when the content itself changes.

/// Whether `raw` (`2.1.326 (Claude Code)`, `codex-cli 0.169.0`, `1.5.10`,
/// `2026.11.13-eacade1`) is at least `min`. Unparseable versions are rejected.
pub fn at_least(raw: &str, min: &str) -> bool {
    match (parts(raw), parts(min)) {
        (Some(version), Some(min)) => version >= min,
        _ => false,
    }
}

fn parts(raw: &str) -> Option<Vec<u32>> {
    let raw = raw.trim();
    let raw = raw.strip_prefix("codex-cli ").unwrap_or(raw);
    let version = raw.split_whitespace().next()?.trim_start_matches('v');
    let numbers = version.split(['-', '+']).next()?.split('.');
    let parts = numbers
        .map(str::parse)
        .collect::<Result<Vec<u32>, _>>()
        .ok()?;
    (parts.len() == 3).then_some(parts)
}

#[cfg(test)]
mod tests {
    use super::at_least;

    #[test]
    fn accepts_the_floor_and_every_newer_build() {
        assert!(at_least("2.1.326 (Claude Code)", "2.1.326"));
        assert!(at_least("2.1.329 (Claude Code)", "2.1.326"));
        assert!(!at_least("2.1.325 (Claude Code)", "2.1.326"));
        assert!(at_least("codex-cli 0.171.0", "0.169.0"));
        assert!(!at_least("codex-cli 0.168.0", "0.169.0"));
        assert!(at_least("1.5.13", "1.5.10"));
        assert!(!at_least("1.5.9", "1.5.10"));
        assert!(at_least("v1.10.0", "1.5.10"));
        assert!(at_least("2026.11.20-abc1234", "2026.11.13"));
        assert!(!at_least("2026.11.12-example", "2026.11.13"));
        assert!(!at_least("unrecognized", "1.0.0"));
        assert!(!at_least("future", "2026.11.13"));
        assert!(!at_least("", "1.0.0"));
    }
}
