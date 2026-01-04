use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// Worker registration record
#[derive(Debug, Clone, FromRow)]
pub struct Worker {
    #[allow(dead_code)]
    pub id: i32,
    #[allow(dead_code)]
    pub environment: String,
    pub endpoint: String,
    #[allow(dead_code)]
    pub enabled: bool,
    #[allow(dead_code)]
    pub last_seen: Option<DateTime<Utc>>,
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub updated_at: DateTime<Utc>,
}

/// Authorized organization record
#[derive(Debug, Clone, FromRow)]
pub struct AuthorizedOrg {
    pub id: i32,
    pub github_org: String,
    pub zones: Vec<String>,
    pub domain_patterns: Vec<String>,
    pub enabled: bool,
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
    #[allow(dead_code)]
    pub updated_at: DateTime<Utc>,
}

impl AuthorizedOrg {
    /// Check if this org is authorized to deploy to a zone
    pub fn can_use_zone(&self, zone: &str) -> bool {
        self.zones.iter().any(|z| z.eq_ignore_ascii_case(zone))
    }

    /// Check if this org is authorized to use a domain
    pub fn can_use_domain(&self, domain: &str) -> bool {
        let domain_lower = domain.to_lowercase();
        self.domain_patterns.iter().any(|pattern| {
            let pattern_lower = pattern.to_lowercase();
            if pattern_lower.starts_with("*.") {
                // Wildcard pattern: *.example.com matches foo.example.com and bar.example.com
                let suffix = &pattern_lower[1..]; // ".example.com"
                domain_lower.ends_with(suffix) && domain_lower.len() > suffix.len()
                    || domain_lower == pattern_lower[2..] // Also match the apex (example.com)
            } else {
                // Exact match
                domain_lower == pattern_lower
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_auth_org(zones: Vec<&str>, domain_patterns: Vec<&str>) -> AuthorizedOrg {
        AuthorizedOrg {
            id: 1,
            github_org: "test-org".to_string(),
            zones: zones.into_iter().map(String::from).collect(),
            domain_patterns: domain_patterns.into_iter().map(String::from).collect(),
            enabled: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_can_use_zone() {
        let auth = make_auth_org(vec!["production", "staging"], vec![]);

        assert!(auth.can_use_zone("production"));
        assert!(auth.can_use_zone("Production")); // Case insensitive
        assert!(auth.can_use_zone("PRODUCTION"));
        assert!(auth.can_use_zone("staging"));
        assert!(!auth.can_use_zone("development"));
    }

    #[test]
    fn test_can_use_domain_exact() {
        let auth = make_auth_org(vec![], vec!["example.com", "test.org"]);

        assert!(auth.can_use_domain("example.com"));
        assert!(auth.can_use_domain("Example.COM")); // Case insensitive
        assert!(auth.can_use_domain("test.org"));
        assert!(!auth.can_use_domain("other.com"));
        assert!(!auth.can_use_domain("sub.example.com")); // No wildcard
    }

    #[test]
    fn test_can_use_domain_wildcard() {
        let auth = make_auth_org(vec![], vec!["*.example.com"]);

        // Wildcard matches subdomains
        assert!(auth.can_use_domain("foo.example.com"));
        assert!(auth.can_use_domain("bar.example.com"));
        assert!(auth.can_use_domain("pr-123.example.com"));

        // Wildcard also matches apex
        assert!(auth.can_use_domain("example.com"));

        // Case insensitive
        assert!(auth.can_use_domain("FOO.Example.COM"));

        // Doesn't match other domains
        assert!(!auth.can_use_domain("example.org"));
        assert!(!auth.can_use_domain("notexample.com"));
    }

    #[test]
    fn test_can_use_domain_multiple_patterns() {
        let auth = make_auth_org(vec![], vec!["*.nullislabs.io", "*.nxm.rs", "nxm.rs"]);

        assert!(auth.can_use_domain("app.nullislabs.io"));
        assert!(auth.can_use_domain("pr-1.nxm.rs"));
        assert!(auth.can_use_domain("nxm.rs"));
        assert!(auth.can_use_domain("www.nxm.rs"));
        assert!(!auth.can_use_domain("other.com"));
    }
}
