//! Native CLI version floors. Integrations name the oldest build they were verified on and
//! accept every newer one: provider CLIs ship often, and an exact pin breaks every person on
//! each release. Readers of native formats still fail closed when the content itself changes.

/// Whether a semver CLI (`2.1.326 (Claude Code)`, `codex-cli 0.169.0`, `1.5.10`) is at least the
/// release `min`. A prerelease of the floor precedes it, as in semver. Unparseable versions fail.
pub fn at_least(raw: &str, min: &str) -> bool {
    match (parts(raw), parts(min)) {
        (Some((version, prerelease)), Some((min, _))) => {
            version > min || (version == min && !prerelease)
        }
        _ => false,
    }
}

/// Whether a dated build (Cursor's `2026.11.13-eacade1`) is at least the day `min`. The suffix is
/// a build hash, not a prerelease, so it never orders builds.
pub fn build_date_at_least(raw: &str, min: &str) -> bool {
    match (parts(raw), parts(min)) {
        (Some((version, _)), Some((min, _))) => version >= min,
        _ => false,
    }
}

/// Three numeric components and whether a `-suffix` follows them. `+build` metadata is ignored.
fn parts(raw: &str) -> Option<(Vec<u32>, bool)> {
    let raw = raw.trim();
    let raw = raw.strip_prefix("codex-cli ").unwrap_or(raw);
    let version = raw.split_whitespace().next()?.trim_start_matches('v');
    let version = version.split('+').next()?;
    let (numbers, suffix) = match version.split_once('-') {
        Some((numbers, suffix)) => (numbers, !suffix.is_empty()),
        None => (version, false),
    };
    let parts = numbers
        .split('.')
        .map(str::parse)
        .collect::<Result<Vec<u32>, _>>()
        .ok()?;
    (parts.len() == 3).then_some((parts, suffix))
}

#[cfg(test)]
mod tests {
    use super::{at_least, build_date_at_least};

    #[test]
    fn accepts_the_floor_and_every_newer_release() {
        assert!(at_least("2.1.326 (Claude Code)", "2.1.326"));
        assert!(at_least("2.1.329 (Claude Code)", "2.1.326"));
        assert!(!at_least("2.1.325 (Claude Code)", "2.1.326"));
        assert!(at_least("codex-cli 0.171.0", "0.169.0"));
        assert!(!at_least("codex-cli 0.168.0", "0.169.0"));
        assert!(at_least("1.5.13", "1.5.10"));
        assert!(!at_least("1.5.9", "1.5.10"));
        assert!(at_least("v1.10.0", "1.5.10"));
        assert!(at_least("1.5.10+build.7", "1.5.10"));
        assert!(!at_least("unrecognized", "1.0.0"));
        assert!(!at_least("", "1.0.0"));
    }

    #[test]
    fn a_prerelease_of_the_floor_precedes_it() {
        assert!(!at_least("codex-cli 0.169.0-alpha.1", "0.169.0"));
        assert!(!at_least("2.1.326-beta (Claude Code)", "2.1.326"));
        assert!(at_least("codex-cli 0.170.0-alpha.1", "0.169.0"));
    }

    #[test]
    fn dated_builds_ignore_their_hash() {
        assert!(build_date_at_least("2026.11.13-eacade1", "2026.11.13"));
        assert!(build_date_at_least("2026.11.20-abc1234", "2026.11.13"));
        assert!(!build_date_at_least("2026.11.12-example", "2026.11.13"));
        assert!(!build_date_at_least("future", "2026.11.13"));
    }
}
