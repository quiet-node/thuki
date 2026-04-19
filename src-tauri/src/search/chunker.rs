//! Split reader-extracted markdown into token-budgeted chunks that carry
//! their source URL and title.
//!
//! The pipeline feeds these chunks to the BM25 chunk-level reranker and then
//! to the sufficiency judge. Chunks are produced at paragraph boundaries when
//! possible (split on blank lines) so semantic units stay intact. Oversized
//! paragraphs fall back to word-group slicing.
//!
//! Token counting approximates one whitespace-separated word as one token.
//! That is intentionally coarse: precise tokenization would couple the chunker
//! to a specific model's tokenizer, while the purpose here is only to keep
//! per-chunk text small enough for the judge's prompt budget.

/// Reader-extracted page, input to `chunk_pages`.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    /// Source URL. Preserved on every chunk derived from this page so the
    /// synthesis prompt can keep citations accurate.
    pub url: String,
    /// Page title (possibly empty). Used for source previews and rerank
    /// context.
    pub title: String,
    /// Extracted markdown body from Trafilatura.
    pub markdown: String,
}

/// Chunk of a page sized to fit within the judge's context window.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// URL of the source page. Identical across sibling chunks.
    pub source_url: String,
    /// Title of the source page. Identical across sibling chunks.
    pub source_title: String,
    /// Chunk text. Guaranteed non-empty and valid UTF-8.
    pub text: String,
}

/// Split `pages` into chunks of roughly `target_tokens` tokens each.
///
/// - Empty pages yield zero chunks.
/// - Paragraphs (blank-line-separated blocks) are preserved intact when
///   possible, merged until the running total exceeds the budget.
/// - Oversized single paragraphs are split by words into pieces of at most
///   `target_tokens` words.
///
/// Complexity: O(N) in total markdown bytes across all pages. No allocation
/// beyond the output `Vec<Chunk>` plus the intermediate paragraph vector per
/// page.
pub fn chunk_pages(pages: &[Page], target_tokens: usize) -> Vec<Chunk> {
    let mut out = Vec::new();
    for p in pages {
        if p.markdown.trim().is_empty() {
            continue;
        }
        for text in split_into_budgeted_blocks(&p.markdown, target_tokens) {
            out.push(Chunk {
                source_url: p.url.clone(),
                source_title: p.title.clone(),
                text,
            });
        }
    }
    out
}

fn split_into_budgeted_blocks(text: &str, target_tokens: usize) -> Vec<String> {
    let paragraphs: Vec<&str> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();

    if paragraphs.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_tokens = 0usize;

    for p in paragraphs {
        let p_tokens = count_tokens(p);
        if p_tokens >= target_tokens {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
                current_tokens = 0;
            }
            for piece in split_paragraph_by_words(p, target_tokens) {
                out.push(piece);
            }
            continue;
        }
        if current_tokens + p_tokens > target_tokens && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            current_tokens = 0;
        }
        if !current.is_empty() {
            current.push_str("\n\n");
        }
        current.push_str(p);
        current_tokens += p_tokens;
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn split_paragraph_by_words(p: &str, target: usize) -> Vec<String> {
    let words: Vec<&str> = p.split_whitespace().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let end = (i + target).min(words.len());
        out.push(words[i..end].join(" "));
        i = end;
    }
    out
}

fn count_tokens(s: &str) -> usize {
    s.split_whitespace().count().max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_page(url: &str, body: &str) -> Page {
        Page {
            url: url.to_string(),
            title: "t".to_string(),
            markdown: body.to_string(),
        }
    }

    #[test]
    fn short_page_yields_single_chunk_preserving_url() {
        let pages = vec![make_page("https://a.com/1", "hello world")];
        let chunks = chunk_pages(&pages, 500);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].source_url, "https://a.com/1");
        assert_eq!(chunks[0].text.trim(), "hello world");
    }

    #[test]
    fn long_page_splits_on_paragraph_boundaries_when_possible() {
        let body = (0..20)
            .map(|i| format!("paragraph {} text", i))
            .collect::<Vec<_>>()
            .join("\n\n");
        let pages = vec![make_page("https://a.com/2", &body)];
        let chunks = chunk_pages(&pages, 20);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert_eq!(c.source_url, "https://a.com/2");
            assert!(!c.text.is_empty());
        }
    }

    #[test]
    fn unicode_boundaries_are_respected() {
        let body = "日本語のテキストです。".repeat(100);
        let pages = vec![make_page("https://a.com/ja", &body)];
        let chunks = chunk_pages(&pages, 30);
        for c in &chunks {
            assert!(c.text.is_char_boundary(0));
            assert!(c.text.is_char_boundary(c.text.len()));
        }
    }

    #[test]
    fn multiple_pages_preserve_source_url_per_chunk() {
        let pages = vec![
            make_page("https://a.com/1", "alpha"),
            make_page("https://b.com/2", "beta content longer text"),
        ];
        let chunks = chunk_pages(&pages, 500);
        let urls: Vec<&str> = chunks.iter().map(|c| c.source_url.as_str()).collect();
        assert!(urls.contains(&"https://a.com/1"));
        assert!(urls.contains(&"https://b.com/2"));
    }

    #[test]
    fn empty_page_yields_no_chunks() {
        let pages = vec![make_page("https://a.com/empty", "")];
        let chunks = chunk_pages(&pages, 500);
        assert!(chunks.is_empty());
    }

    #[test]
    fn whitespace_only_page_yields_no_chunks() {
        let pages = vec![make_page("https://a.com/ws", "   \n\n   ")];
        let chunks = chunk_pages(&pages, 500);
        assert!(chunks.is_empty());
    }

    #[test]
    fn oversized_paragraph_is_word_split() {
        let body = "word ".repeat(50); // 50 words, one paragraph
        let pages = vec![make_page("https://a.com/big", &body)];
        let chunks = chunk_pages(&pages, 10);
        assert_eq!(chunks.len(), 5);
        for c in &chunks {
            assert!(count_tokens(&c.text) <= 10);
        }
    }

    #[test]
    fn all_whitespace_paragraphs_yield_empty_vec() {
        // All paragraphs are whitespace-only, so `paragraphs.is_empty()` is
        // true after the filter and the early-return on line 74 fires.
        let result = split_into_budgeted_blocks("\n\n   \n\n\t\n\n", 500);
        assert!(result.is_empty());
    }

    #[test]
    fn oversized_paragraph_after_small_paragraphs_flushes_accumulator() {
        // Two small paragraphs accumulate in `current`, then an oversized
        // paragraph arrives. The flush branch (lines 85-86) pushes `current`
        // before word-splitting the oversized paragraph.
        let small1 = "alpha beta"; // 2 tokens
        let small2 = "gamma delta"; // 2 tokens
                                    // 30 words: oversized relative to target_tokens=20.
        let oversized = (0..30)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let body = format!("{small1}\n\n{small2}\n\n{oversized}");
        let chunks = split_into_budgeted_blocks(&body, 20);
        // First chunk: the two small paragraphs flushed together.
        assert!(chunks[0].contains("alpha beta"));
        assert!(chunks[0].contains("gamma delta"));
        // Remaining chunks: word-split pieces of the oversized paragraph.
        assert!(chunks.len() >= 2);
        let oversized_text: String = chunks[1..].join(" ");
        for w in 0..30 {
            assert!(oversized_text.contains(&format!("word{w}")));
        }
    }
}
