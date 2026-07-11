//! Static, compiled-in domain-credibility list for the keyless engine tier.
//!
//! A curated list of registrable domains is embedded at build time (see
//! `credibility_domains.txt`) and parsed once into three sets: `drop` domains
//! (individually verified hoax and impostor sources, hard-removed before rank
//! fusion), `penalize` domains (bulk-imported SEO-spam and encyclopedia-copycat
//! clusters, given a soft rank penalty in fusion), and `boost` domains
//! (encyclopedic and primary-reference sources, promoted in fusion). The engine
//! fusion step consults [`classify_domain`] to bias its ranking; this module
//! owns only the data and the classification, never the fusion math.
//!
//! The list is a defensive quality signal, not a security boundary: the penalize
//! and boost sets are advisory rank nudges, and only the small, hand-verified
//! drop set removes results outright.

use std::collections::HashSet;
use std::sync::LazyLock;

/// The credibility verdict for a single host, consumed by the engine fusion step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainClass {
    /// Individually verified hoax or impostor domain: hard-removed before fusion.
    Drop,
    /// Bulk-imported spam or copycat domain: soft rank penalty in fusion.
    Penalize,
    /// Encyclopedic or primary-reference domain: promoted in fusion.
    Boost,
    /// Not on any list: fused on its native rank with no adjustment.
    Neutral,
}

/// The three parsed domain sets, keyed by class.
struct CredibilitySets {
    /// Hard-drop domains.
    drop: HashSet<String>,
    /// Soft-penalty domains.
    penalize: HashSet<String>,
    /// Rank-boost domains.
    boost: HashSet<String>,
}

/// The embedded credibility list, parsed once on first access. The `include_str!`
/// resolves at compile time, so the data ships inside the binary with no runtime
/// file I/O.
static SETS: LazyLock<CredibilitySets> = LazyLock::new(|| {
    let (drop, penalize, boost) = parse_credibility(include_str!("credibility_domains.txt"));
    CredibilitySets {
        drop,
        penalize,
        boost,
    }
});

/// Parses the embedded credibility text into `(drop, penalize, boost)` domain
/// sets. The format is line-oriented: a line reading exactly `# drop`,
/// `# penalize`, or `# boost` (ignoring surrounding `#` and whitespace) switches
/// the active section; any other `#` line is a courtesy source comment and is
/// skipped; a blank line is skipped; a domain line before any section header, or
/// one carrying internal whitespace, is treated as malformed and skipped. Every
/// accepted domain is lowercased. The parser is pure and total: no input line can
/// panic, so a malformed embedded file degrades to smaller sets rather than a
/// crash. Taking the text as an argument keeps the parser testable on synthetic
/// input, independent of the real embedded file's contents.
fn parse_credibility(text: &str) -> (HashSet<String>, HashSet<String>, HashSet<String>) {
    let mut drop = HashSet::new();
    let mut penalize = HashSet::new();
    let mut boost = HashSet::new();
    let mut section: Option<&mut HashSet<String>> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            match rest.trim() {
                "drop" => section = Some(&mut drop),
                "penalize" => section = Some(&mut penalize),
                "boost" => section = Some(&mut boost),
                // Any other comment (courtesy source credit, file header) is data
                // for humans, not the parser.
                _ => {}
            }
            continue;
        }
        // A domain line only counts inside a section and must be a bare host.
        let Some(set) = section.as_mut() else {
            continue;
        };
        if line.contains(char::is_whitespace) {
            continue;
        }
        set.insert(line.to_ascii_lowercase());
    }
    (drop, penalize, boost)
}

/// Classifies `host` against the three sets, matching the host itself and each of
/// its parent registrable suffixes so a subdomain inherits its parent's verdict
/// (a host `www.foo.example.com` matches a listed `example.com`). Because the
/// dot-separated suffixes are all distinct, "longest match" only ever selects
/// among suffixes of the same class; when suffixes fall into different classes
/// the verdict is resolved by precedence [`DomainClass::Drop`] over
/// [`DomainClass::Penalize`] over [`DomainClass::Boost`], so the most protective
/// verdict wins (safety first). A host on no list is [`DomainClass::Neutral`].
/// Split from [`classify_domain`] so the precedence logic is testable against
/// synthetic sets.
fn classify_in(
    host: &str,
    drop: &HashSet<String>,
    penalize: &HashSet<String>,
    boost: &HashSet<String>,
) -> DomainClass {
    let host = host.to_ascii_lowercase();
    let mut matched = DomainClass::Neutral;
    let bytes = host.as_bytes();
    // Walk every suffix that begins at a label boundary: the whole host, then the
    // string after each dot. The whole-host case is the first iteration (offset 0).
    for (idx, &b) in bytes.iter().enumerate() {
        let is_boundary = idx == 0 || (b == b'.' && idx + 1 < bytes.len());
        if !is_boundary {
            continue;
        }
        let suffix = if idx == 0 {
            &host[..]
        } else {
            &host[idx + 1..]
        };
        if drop.contains(suffix) {
            // Drop is the most protective verdict; nothing can override it.
            return DomainClass::Drop;
        }
        if penalize.contains(suffix) {
            matched = DomainClass::Penalize;
        } else if boost.contains(suffix) && matched == DomainClass::Neutral {
            // Boost only wins when no penalize suffix has matched (Penalize > Boost).
            matched = DomainClass::Boost;
        }
    }
    matched
}

/// Classifies `host` against the embedded credibility list. Thin wrapper over
/// [`classify_in`] that supplies the process-wide parsed [`SETS`]. Returns
/// [`DomainClass::Neutral`] for any host not on a list, which is the common case.
pub fn classify_domain(host: &str) -> DomainClass {
    classify_in(host, &SETS.drop, &SETS.penalize, &SETS.boost)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small synthetic list exercising every parser branch: the three section
    /// headers, a courtesy comment, a blank line, an uppercase domain (lowercased
    /// on insert), a domain before any section (skipped), and a malformed
    /// internal-whitespace domain (skipped).
    const SYNTHETIC: &str = "\
orphan-before-section.com
# drop
# cluster: courtesy comment, ignored

Evil.Example.COM
bad domain with spaces.com
# penalize
spam.example
# boost
good.example
";

    #[test]
    fn parse_switches_sections_and_normalizes() {
        let (drop, penalize, boost) = parse_credibility(SYNTHETIC);
        // Uppercase domain lowercased, courtesy comment and blank skipped.
        assert!(drop.contains("evil.example.com"));
        assert_eq!(drop.len(), 1);
        assert!(penalize.contains("spam.example"));
        assert_eq!(penalize.len(), 1);
        assert!(boost.contains("good.example"));
        assert_eq!(boost.len(), 1);
    }

    #[test]
    fn parse_skips_orphan_and_malformed_lines() {
        let (drop, penalize, boost) = parse_credibility(SYNTHETIC);
        // A domain before the first section header is dropped.
        assert!(!drop.contains("orphan-before-section.com"));
        assert!(!penalize.contains("orphan-before-section.com"));
        assert!(!boost.contains("orphan-before-section.com"));
        // A domain line carrying internal whitespace is malformed and skipped.
        assert!(!drop.iter().any(|d| d.contains("bad domain")));
    }

    #[test]
    fn classify_exact_match_per_class() {
        let (drop, penalize, boost) = parse_credibility(SYNTHETIC);
        assert_eq!(
            classify_in("evil.example.com", &drop, &penalize, &boost),
            DomainClass::Drop
        );
        assert_eq!(
            classify_in("spam.example", &drop, &penalize, &boost),
            DomainClass::Penalize
        );
        assert_eq!(
            classify_in("good.example", &drop, &penalize, &boost),
            DomainClass::Boost
        );
    }

    #[test]
    fn classify_matches_subdomain_via_suffix() {
        let (drop, penalize, boost) = parse_credibility(SYNTHETIC);
        // A deep subdomain inherits its parent registrable domain's verdict.
        assert_eq!(
            classify_in("www.foo.good.example", &drop, &penalize, &boost),
            DomainClass::Boost
        );
    }

    #[test]
    fn classify_unknown_host_is_neutral() {
        let (drop, penalize, boost) = parse_credibility(SYNTHETIC);
        assert_eq!(
            classify_in("nothing-here.test", &drop, &penalize, &boost),
            DomainClass::Neutral
        );
        // The bare public suffix of a listed host is not itself listed.
        assert_eq!(
            classify_in("example", &drop, &penalize, &boost),
            DomainClass::Neutral
        );
    }

    #[test]
    fn classify_precedence_drop_over_penalize_over_boost() {
        // Craft conflicting suffixes so the precedence rule is the deciding factor.
        let mut drop = HashSet::new();
        let mut penalize = HashSet::new();
        let mut boost = HashSet::new();
        drop.insert("evil.site.com".to_string());
        penalize.insert("site.com".to_string());
        boost.insert("good.site.com".to_string());
        // evil.site.com is Drop; site.com is Penalize -> Drop wins (most protective).
        assert_eq!(
            classify_in("evil.site.com", &drop, &penalize, &boost),
            DomainClass::Drop
        );
        // good.site.com is Boost but its parent site.com is Penalize -> Penalize
        // wins over Boost (safety first).
        assert_eq!(
            classify_in("good.site.com", &drop, &penalize, &boost),
            DomainClass::Penalize
        );
    }

    #[test]
    fn embedded_file_parses_to_expected_sets() {
        // The real compiled-in list classifies a known member of each section and
        // carries the expected approximate volume, proving the include_str! data
        // parsed rather than silently emptying.
        assert_eq!(classify_domain("wikipedia.org"), DomainClass::Boost);
        assert_eq!(classify_domain("en.wikipedia.org"), DomainClass::Boost);
        assert_eq!(classify_domain("now8news.com"), DomainClass::Drop);
        assert_eq!(classify_domain("9to5answer.com"), DomainClass::Penalize);
        assert_eq!(
            classify_domain("some-random-host.test"),
            DomainClass::Neutral
        );
        assert!(
            SETS.drop.len() + SETS.penalize.len() > 200,
            "drop+penalize should be the full downrank list"
        );
        assert!(SETS.boost.len() > 50, "boost list should be populated");
    }

    #[test]
    fn embedded_file_classifies_live_observed_hoax_and_spam_additions() {
        // 2026-07-11 smoke-session additions: individually verified via live
        // search-trace inspection rather than a bulk-imported list. mediamass.net
        // is listed at the registrable-domain level so the observed subdomain
        // en.mediamass.net inherits Drop via suffix matching.
        assert_eq!(classify_domain("en.mediamass.net"), DomainClass::Drop);
        assert_eq!(classify_domain("mediamass.net"), DomainClass::Drop);
        assert_eq!(classify_domain("www.newsunzip.com"), DomainClass::Drop);
        assert_eq!(
            classify_domain("www.current-affairs.org"),
            DomainClass::Drop
        );
        assert_eq!(classify_domain("grizzlybulls.com"), DomainClass::Penalize);
        assert_eq!(
            classify_domain("agecalculator.iamrohit.in"),
            DomainClass::Penalize
        );
        assert_eq!(classify_domain("daycalculator.com"), DomainClass::Penalize);
        // Explicitly excluded per product-owner review: staleness is handled by
        // recency ranking, not credibility.
        assert_eq!(classify_domain("insiderpaper.com"), DomainClass::Neutral);
        assert_eq!(classify_domain("plisio.net"), DomainClass::Neutral);
    }
}
