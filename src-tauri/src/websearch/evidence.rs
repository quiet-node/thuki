//! Post-fetch evidence filters for freshness and live-price turns.
//!
//! After BM25 ranking (and optional recency reordering), the engine tier still
//! holds SEO scrapes with old numbers and official quote pages that extracted
//! as numberless marketing chrome. This module applies pure filters used only
//! when the turn's signals ask for them:
//!
//! 1. **Stale path years** (freshness turns): drop chunks whose URL path
//!    contains `/YYYY/` with a year older than [`STALE_PATH_YEAR_LAG`] full
//!    years (e.g. `/2020/` in 2026). Archive paths are a common SEO pattern
//!    for "today" price articles that still rank on lexical BM25.
//! 2. **Price numeric utility** (price-intent turns): keep only chunks that
//!    carry a price-like digit run. If none do, return empty so the
//!    orchestrator can refuse rather than let the writer invent confidence
//!    from numberless shells while a scraper with "80 triệu" would otherwise
//!    have won.
//! 3. **Price magnitude consensus** (price-intent turns, after numeric utility):
//!    when multiple numeric chunks disagree by an order of magnitude or more
//!    (e.g. one page quotes SJC miếng at `14.350.000` while peers quote
//!    `145.500.000`), drop the minority magnitude cluster so the writer cannot
//!    lead with a self-consistent but 10×-wrong source. Digits-in-source cite
//!    audit cannot catch this class: the bad page agrees with itself.
//!
//! All filters are pure over their inputs, allocate at most the size of the
//! input chunk list, and never panic on hostile URLs or empty text.

use crate::config::defaults::{
    PRICE_LIKE_MIN_DIGIT_RUN, PRICE_MAGNITUDE_MIN_PRIMARY, PRICE_MAGNITUDE_RATIO,
    STALE_PATH_YEAR_LAG,
};
use crate::websearch::rank::ScoredChunk;

/// Applies freshness and price-intent evidence filters to ranked chunks.
///
/// Order is load-bearing: drop multi-year-stale path archives first (so a 2020
/// page with numbers cannot survive a price filter alone), then apply the
/// price numeric utility gate, then cross-source magnitude consensus. Pure
/// and total.
///
/// @param chunks - BM25 (and maybe recency) survivors, best-first.
/// @param freshness - Turn carries a freshness signal (recency path armed).
/// @param price_intent - Question is a price/quote ask.
/// @param now_year - Current UTC calendar year for path-year comparison.
/// @returns Filtered chunks; empty means the orchestrator should treat the
///   engine tier as "found nothing usable" for this evidence bar.
pub(crate) fn filter_evidence_chunks(
    chunks: Vec<ScoredChunk>,
    freshness: bool,
    price_intent: bool,
    now_year: u32,
) -> Vec<ScoredChunk> {
    let chunks = if freshness {
        filter_stale_path_years(chunks, now_year)
    } else {
        chunks
    };
    if price_intent {
        let with_nums = filter_price_numeric_utility(chunks);
        filter_price_magnitude_outliers(with_nums)
    } else {
        chunks
    }
}

/// Drops chunks whose URL path embeds a calendar year at least
/// [`STALE_PATH_YEAR_LAG`] full years older than `now_year`.
///
/// Only `/YYYY/` path segments of four digits starting with `20` are
/// considered (modern web archive paths). Query strings and fragments are
/// ignored by scanning the path portion only. Chunks with no such segment
/// always survive.
fn filter_stale_path_years(chunks: Vec<ScoredChunk>, now_year: u32) -> Vec<ScoredChunk> {
    let cutoff = now_year.saturating_sub(STALE_PATH_YEAR_LAG);
    chunks
        .into_iter()
        .filter(|c| {
            if let Some(year) = path_embedded_year(&c.url) {
                // Keep years newer than the cutoff (e.g. 2025–2026 when lag=2
                // and now=2026). Drop older archive paths.
                year > cutoff
            } else {
                true
            }
        })
        .collect()
}

/// For a price-intent turn: keep only chunks that contain a price-like
/// number. If **no** chunk has one, return empty (refuse path) rather than
/// returning numberless marketing pages that lose to scrapers on the next
/// rank pass.
fn filter_price_numeric_utility(chunks: Vec<ScoredChunk>) -> Vec<ScoredChunk> {
    let with_nums: Vec<ScoredChunk> = chunks
        .into_iter()
        .filter(|c| has_price_like_number(&c.text))
        .collect();
    with_nums
}

/// Drops price chunks whose primary quote sits an order of magnitude away
/// from the peer-majority (or score-weighted) cluster.
///
/// Only activates when at least two chunks each expose a primary price at or
/// above [`PRICE_MAGNITUDE_MIN_PRIMARY`] and the max/min primary ratio is at
/// least [`PRICE_MAGNITUDE_RATIO`]. Otherwise the list is returned unchanged
/// (single-source and near-agreement cases must not invent consensus).
///
/// Pure and total: never panics; allocates at most one output vector of the
/// input length.
fn filter_price_magnitude_outliers(chunks: Vec<ScoredChunk>) -> Vec<ScoredChunk> {
    if chunks.len() < 2 {
        return chunks;
    }
    // `primary_price_value` already floors at PRICE_MAGNITUDE_MIN_PRIMARY.
    let primaries: Vec<(usize, f64)> = chunks
        .iter()
        .enumerate()
        .filter_map(|(i, c)| primary_price_value(&c.text).map(|v| (i, v)))
        .collect();
    if primaries.len() < 2 {
        return chunks;
    }
    let min_p = primaries
        .iter()
        .map(|(_, v)| *v)
        .fold(f64::INFINITY, f64::min);
    let max_p = primaries.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max);
    // Primaries are always > 0 by construction; ratio short-circuit is the
    // near-agreement guard (bid/ask within the same order of magnitude).
    if max_p / min_p < PRICE_MAGNITUDE_RATIO {
        return chunks;
    }
    // Bucket by floor(log10). Majority count wins. On a count tie (common
    // 1-vs-1 wrong-scale case), prefer the *higher* magnitude bucket: a
    // high-scoring chỉ-scale page (14.35M) must not beat a real miếng quote
    // (145.5M) just because BM25 liked the long wrong table more.
    let mut bucket_count: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
    for &(_, v) in &primaries {
        *bucket_count.entry(magnitude_bucket(v)).or_insert(0) += 1;
    }
    // `primaries` non-empty ⇒ `bucket_count` non-empty ⇒ `max_by` always Some.
    let win = *bucket_count
        .iter()
        .max_by(|(b1, c1), (b2, c2)| c1.cmp(c2).then_with(|| b1.cmp(b2)))
        .expect("bucket_count non-empty when primaries >= 2")
        .0;
    let keep: std::collections::HashSet<usize> = primaries
        .iter()
        .filter(|(_, v)| magnitude_bucket(*v) == win)
        .map(|(i, _)| *i)
        .collect();
    // Drop loser-bucket primaries; keep ancillary chunks (small % deltas, etc.)
    // that never exposed a competing primary above the floor.
    chunks
        .into_iter()
        .enumerate()
        .filter(|(i, c)| {
            if keep.contains(i) {
                return true;
            }
            // Drop loser-bucket primaries only.
            !matches!(
                primary_price_value(&c.text),
                Some(v) if v >= PRICE_MAGNITUDE_MIN_PRIMARY
            )
        })
        .map(|(_, c)| c)
        .collect()
}

/// Order-of-magnitude bucket for a positive price: `floor(log10(v))`.
///
/// Callers only pass primaries from [`primary_price_value`] (always `> 0`).
fn magnitude_bucket(v: f64) -> i32 {
    v.log10().floor() as i32
}

/// Largest price-like numeric value in `text`, preferring multi-digit runs that
/// look like money (at least [`PRICE_LIKE_MIN_DIGIT_RUN`] digits after stripping
/// grouping separators). Returns `None` when nothing qualifies.
///
/// Handles VN/EU multi-dot thousands (`144.500.000`) and EN comma thousands
/// (`144,500,000`) by stripping separators when every group is three digits.
/// Pure and total.
pub(crate) fn primary_price_value(text: &str) -> Option<f64> {
    // Collapse Unicode thousands separators (thin space U+202F, NBSP) so the
    // ASCII scanner only has to handle `.` / `,` grouping. Plain ASCII spaces
    // stay: "9999 145.500.000" must remain two values.
    let normalized: String = text
        .chars()
        .map(|c| {
            if c == '\u{202f}' || c == '\u{00a0}' {
                '\0'
            } else {
                c
            }
        })
        .filter(|c| *c != '\0')
        .collect();
    let mut best: Option<f64> = None;
    let bytes = normalized.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        // Consume digits and `.` / `,` grouping separators only.
        while i < bytes.len() {
            let b = bytes[i];
            let is_digit = b.is_ascii_digit();
            let is_group_sep =
                (b == b'.' || b == b',') && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit();
            if is_digit || is_group_sep {
                i += 1;
            } else {
                break;
            }
        }
        let raw = &normalized[start..i];
        if let Some(v) = parse_grouped_number(raw) {
            if v >= PRICE_MAGNITUDE_MIN_PRIMARY {
                best = Some(best.map_or(v, |b| b.max(v)));
            }
        }
    }
    best
}

/// Parses a digit run that may contain `.` / `,` / spaces as thousands
/// separators into an `f64`. Returns `None` when digit count is below
/// [`PRICE_LIKE_MIN_DIGIT_RUN`] or the form is not a clean integer group.
fn parse_grouped_number(raw: &str) -> Option<f64> {
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < PRICE_LIKE_MIN_DIGIT_RUN {
        return None;
    }
    // Reject pure years / short list indices already gated by min primary.
    digits.parse::<f64>().ok()
}

/// True when `text` carries a run of at least [`PRICE_LIKE_MIN_DIGIT_RUN`]
/// consecutive ASCII digits (or a `%` figure). Fullwidth digits are not
/// required here: the gold-price regression sources used ASCII runs.
pub(crate) fn has_price_like_number(text: &str) -> bool {
    if text.contains('%') {
        return true;
    }
    let mut run = 0usize;
    for c in text.chars() {
        if c.is_ascii_digit() {
            run += 1;
            if run >= PRICE_LIKE_MIN_DIGIT_RUN {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

/// First `/20XX/` four-digit year embedded in the URL path, if any.
///
/// Scans only the path (before `?` / `#`). Pure and total: malformed URLs
/// simply yield `None`.
pub(crate) fn path_embedded_year(url: &str) -> Option<u32> {
    // `split` always yields at least one segment (the full string when no
    // separator), so `next()` is never None.
    let path = url
        .split(['?', '#'])
        .next()
        .expect("split yields one segment");
    let bytes = path.as_bytes();
    let mut i = 0;
    // Need at least `/20xx` (5 bytes). The sixth byte, when present, must be
    // a path separator or hyphen so we do not match `/20000` or `/20xxfoo`.
    while i + 5 <= bytes.len() {
        if bytes[i] == b'/'
            && bytes[i + 1] == b'2'
            && bytes[i + 2] == b'0'
            && bytes[i + 3].is_ascii_digit()
            && bytes[i + 4].is_ascii_digit()
            && (i + 5 == bytes.len() || bytes[i + 5] == b'/' || bytes[i + 5] == b'-')
        {
            let y = (bytes[i + 1] - b'0') as u32 * 1000
                + (bytes[i + 2] - b'0') as u32 * 100
                + (bytes[i + 3] - b'0') as u32 * 10
                + (bytes[i + 4] - b'0') as u32;
            return Some(y);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::websearch::rank::ScoredChunk;

    fn chunk(url: &str, text: &str) -> ScoredChunk {
        ScoredChunk {
            url: url.into(),
            title: "t".into(),
            text: text.into(),
            score: 1.0,
        }
    }

    // ── path_embedded_year ───────────────────────────────────────────────────

    #[test]
    fn path_year_reads_archive_segment() {
        assert_eq!(
            path_embedded_year(
                "https://globaleasyforex.com/blog/2020/05/gia-vang-giam-hon-20-do-la/"
            ),
            Some(2020)
        );
    }

    #[test]
    fn path_year_none_without_year_segment() {
        assert_eq!(
            path_embedded_year("https://www.24h.com.vn/gia-vang-hom-nay-c425.html"),
            None
        );
    }

    #[test]
    fn path_year_accepts_year_at_path_end_or_hyphen_date() {
        assert_eq!(
            path_embedded_year("https://archive.example/posts/2021"),
            Some(2021)
        );
        assert_eq!(
            path_embedded_year("https://news.example/2022-06-01/story"),
            Some(2022)
        );
    }

    #[test]
    fn path_year_ignores_query_string_noise() {
        assert_eq!(path_embedded_year("https://example.com/page?y=2020"), None);
    }

    // ── stale path years ─────────────────────────────────────────────────────

    #[test]
    fn filter_stale_drops_multi_year_old_path() {
        let chunks = vec![
            chunk("https://x.com/blog/2020/05/old/", "Giá vàng 1.700 USD"),
            chunk("https://www.pnj.com.vn/site/gia-vang", "SJC 144 triệu"),
        ];
        let out = filter_stale_path_years(chunks, 2026);
        assert_eq!(out.len(), 1);
        assert!(out[0].url.contains("pnj.com.vn"));
    }

    #[test]
    fn filter_stale_keeps_recent_path_year() {
        let chunks = vec![chunk("https://news.example/2025/12/gold/", "gold 4000")];
        // cutoff = 2026 - 2 = 2024; 2025 > 2024 keeps.
        let out = filter_stale_path_years(chunks, 2026);
        assert_eq!(out.len(), 1);
    }

    // ── price numeric utility ────────────────────────────────────────────────

    #[test]
    fn has_price_like_number_requires_digit_run() {
        assert!(has_price_like_number("SJC 80 triệu đồng/lượng"));
        assert!(has_price_like_number("under 1.700 USD"));
        assert!(!has_price_like_number(
            "Cập nhật giá vàng hôm nay mới nhất PNJ SJC"
        ));
        assert!(has_price_like_number("up 12%"));
    }

    #[test]
    fn price_utility_keeps_only_numeric_chunks_when_any_exist() {
        let chunks = vec![
            chunk(
                "https://spam.example/x",
                "Giá vàng hôm nay 23/3: Vàng SJC đứng ở mức 80 triệu",
            ),
            chunk(
                "https://www.24h.com.vn/gia-vang",
                "Giá vàng hôm nay các thương hiệu Sjc mới nhất",
            ),
        ];
        let out = filter_price_numeric_utility(chunks);
        assert_eq!(out.len(), 1);
        assert!(out[0].text.contains("80"));
    }

    #[test]
    fn price_utility_empties_when_no_chunk_has_numbers() {
        // Official pages extracted as marketing chrome: refuse rather than
        // hand the writer numberless context that scrapers will outrank.
        let chunks = vec![
            chunk(
                "https://www.24h.com.vn/gia-vang",
                "Giá vàng hôm nay các thương hiệu Sjc mới nhất",
            ),
            chunk(
                "https://www.pnj.com.vn/site/gia-vang",
                "Cập Nhật Mới Nhất Bảng Giá Vàng Hôm Nay",
            ),
        ];
        let out = filter_price_numeric_utility(chunks);
        assert!(out.is_empty());
    }

    // ── combined filter ──────────────────────────────────────────────────────

    #[test]
    fn gold_smoke_regression_drops_spam_archive_and_numberless() {
        // Reconstruct the 2026-07-14 failure set after credibility would have
        // dropped blogdanica: stale 2020 + empty official shells must not
        // leave a single confident price-bearing scraper as sole survivor
        // unless it has numbers AND a non-stale path (the 2020 URL dies here).
        let chunks = vec![
            chunk(
                "https://globaleasyforex.com/blog/2020/05/gia-vang/",
                "Giá vàng kỳ hạn giảm mạnh hơn 20 đô la, dưới 1.700 đô la hôm nay",
            ),
            chunk(
                "https://www.24h.com.vn/gia-vang-hom-nay-c425.html",
                "Giá vàng hôm nay, hôm qua trong nước và thế giới mới nhất",
            ),
            chunk(
                "https://www.pnj.com.vn/site/gia-vang",
                "Cập Nhật Mới Nhất Bảng Giá Vàng Hôm Nay PNJ SJC",
            ),
        ];
        let out = filter_evidence_chunks(chunks, true, true, 2026);
        // Stale 2020 dropped; remaining are numberless → empty refuse path.
        assert!(out.is_empty());
    }

    #[test]
    fn gold_smoke_keeps_live_numeric_official_page() {
        let chunks = vec![
            chunk(
                "https://globaleasyforex.com/blog/2020/05/old/",
                "1.700 USD hôm nay",
            ),
            chunk(
                "https://www.pnj.com.vn/site/gia-vang",
                "Giá SJC mua vào 144.500.000 bán ra 147.500.000 đồng/lượng",
            ),
        ];
        let out = filter_evidence_chunks(chunks, true, true, 2026);
        assert_eq!(out.len(), 1);
        assert!(out[0].text.contains("144"));
    }

    #[test]
    fn non_price_non_fresh_pass_through_unchanged() {
        let chunks = vec![chunk(
            "https://en.wikipedia.org/wiki/Gold",
            "Gold is a chemical element",
        )];
        let out = filter_evidence_chunks(chunks.clone(), false, false, 2026);
        assert_eq!(out, chunks);
    }

    #[test]
    fn fresh_non_price_only_drops_stale_paths() {
        let chunks = vec![
            chunk("https://x.com/blog/2020/05/old/", "ancient news text"),
            chunk("https://x.com/blog/live/", "current text here"),
        ];
        let out = filter_evidence_chunks(chunks, true, false, 2026);
        assert_eq!(out.len(), 1);
        assert!(out[0].url.contains("live"));
    }

    #[test]
    fn price_non_fresh_still_requires_numbers() {
        // Price intent without freshness (should not happen in prod once
        // is_volatile includes price markers, but the filter is independent).
        let chunks = vec![chunk(
            "https://www.pnj.com.vn/site/gia-vang",
            "Bảng giá vàng hôm nay mới nhất",
        )];
        let out = filter_evidence_chunks(chunks, false, true, 2026);
        assert!(out.is_empty());
    }

    // ── price magnitude consensus ────────────────────────────────────────────

    #[test]
    fn primary_price_value_reads_vn_grouped_millions() {
        assert_eq!(
            primary_price_value("mua vào 14.350.000 đ/lượng bán 14.850.000"),
            Some(14_850_000.0)
        );
        assert_eq!(
            primary_price_value("SJC Miếng 145.500.000▲ 148.500.000"),
            Some(148_500_000.0)
        );
    }

    #[test]
    fn magnitude_filter_drops_10x_outlier_when_peers_agree() {
        // 2026-07-15 smoke: webgia 14.35M "miếng" vs giavangnay 145.5M board.
        // Two correct peers + one chỉ-scale page: drop the 1e7 cluster.
        let chunks = vec![
            ScoredChunk {
                url: "https://webgia.vn/vang-mieng-sjc".into(),
                title: "wrong scale".into(),
                text: "VÀNG MIẾNG SJC mua vào 14.350.000 bán ra 14.850.000 đ/lượng".into(),
                score: 10.0, // high BM25, still wrong
            },
            ScoredChunk {
                url: "https://giavangnay.com/".into(),
                title: "board".into(),
                text: "SJC Miếng 9999 145.500.000 148.500.000 VNĐ/lượng".into(),
                score: 3.0,
            },
            ScoredChunk {
                url: "https://www.pnj.com.vn/site/gia-vang".into(),
                title: "pnj".into(),
                text: "Giá SJC mua vào 144.500.000 bán ra 147.500.000 đồng/lượng".into(),
                score: 2.0,
            },
        ];
        let out = filter_price_magnitude_outliers(chunks);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|c| !c.url.contains("webgia")));
        assert!(out.iter().any(|c| c.text.contains("145")));
    }

    #[test]
    fn magnitude_filter_noop_when_prices_agree() {
        let chunks = vec![
            chunk("https://a.example/", "SJC 145.500.000 mua 148.500.000 bán"),
            chunk("https://b.example/", "SJC 144.500.000 mua 147.500.000 bán"),
        ];
        let out = filter_price_magnitude_outliers(chunks.clone());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn magnitude_filter_noop_on_single_chunk() {
        let chunks = vec![chunk("https://webgia.vn/x", "14.350.000 đ/lượng")];
        let out = filter_price_magnitude_outliers(chunks.clone());
        assert_eq!(out, chunks);
    }

    #[test]
    fn gold_mieng_smoke_combined_keeps_145m_cluster() {
        let chunks = vec![
            ScoredChunk {
                url: "https://webgia.vn/vang-mieng-sjc".into(),
                title: "t".into(),
                text: "15/07/2026 09:02 14.350.000 14.850.000 500.000".into(),
                score: 9.0,
            },
            ScoredChunk {
                url: "https://giavangnay.com/".into(),
                title: "t".into(),
                text: "SJC Miếng 9999 145.500.000 148.500.000".into(),
                score: 4.0,
            },
            ScoredChunk {
                url: "https://example.com/delta".into(),
                title: "t".into(),
                text: "tăng 0,70% so với phiên trước".into(),
                score: 1.0,
            },
        ];
        let out = filter_evidence_chunks(chunks, true, true, 2026);
        assert!(out.iter().any(|c| c.text.contains("145.500")));
        assert!(out.iter().all(|c| !c.url.contains("webgia")));
    }

    #[test]
    fn magnitude_filter_noop_when_only_one_primary_qualifies() {
        // Two chunks, but only one has a primary above the floor: no consensus.
        let chunks = vec![
            chunk("https://a.example/", "up 12% today"),
            chunk("https://b.example/", "SJC 145.500.000 đ/lượng"),
        ];
        let out = filter_price_magnitude_outliers(chunks.clone());
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn primary_price_reads_thin_space_thousands_and_commas() {
        // U+202F thin space (common in writer/source paste) + EN comma groups.
        let thin = format!("mua 144\u{202f}500\u{202f}000 bán");
        assert_eq!(primary_price_value(&thin), Some(144_500_000.0));
        assert_eq!(primary_price_value("spot 1,234,567 USD"), Some(1_234_567.0));
        assert_eq!(primary_price_value("only 9"), None);
        assert_eq!(primary_price_value("no digits here"), None);
    }
}
