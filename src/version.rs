//! Release version parsing and comparison.
//!
//! This build has no self-update or background version checks; the type is
//! kept for local comparisons such as plugin minimum-version requirements and
//! release-notes bookkeeping.

/// Parsed semver version for comparison.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.strip_prefix('v').unwrap_or(s);
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        Some(Self {
            major: parts[0].parse().ok()?,
            minor: parts[1].parse().ok()?,
            patch: parts[2].parse().ok()?,
        })
    }

    pub fn current() -> Self {
        Self::parse(crate::build_info::BASE_VERSION).expect("invalid CARGO_PKG_VERSION")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_orders_versions() {
        let older = Version::parse("v0.9.5").unwrap();
        let newer = Version::parse("0.10.0").unwrap();
        assert!(older < newer);
        assert_eq!(newer.to_string(), "0.10.0");
        assert!(Version::parse("1.2").is_none());
        assert!(Version::parse("1.2.x").is_none());
    }
}
