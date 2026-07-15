//! Post-fetch evidence filters for freshness and live-price turns.
//!
//! After BM25 ranking (and optional recency reordering), the engine tier still
//! holds SEO scrapes with old numbers and official quote pages that extracted
//! as numberless marketing chrome. This module applies two pure filters used
//! only when the turn's signals ask for them:
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
//!
//! Both filters are pure over their inputs, allocate at most the size of the
//! input chunk list, and never panic on hostile URLs or empty text.

use crate::config::defaults::{PRICE_LIKE_MIN_DIGIT_RUN, STALE_PATH_YEAR_LAG};
use crate::websearch::rank::ScoredChunk;

/// Applies freshness and price-intent evidence filters to ranked chunks.
///
/// Order is load-bearing: drop multi-year-stale path archives first (so a 2020
/// page with numbers cannot survive a price filter alone), then apply the
/// price numeric utility gate. Pure and total.
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
        filter_price_numeric_utility(chunks)
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
}
