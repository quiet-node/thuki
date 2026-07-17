/*!
 * Minimal, panic-safe GGUF metadata reader.
 *
 * The reasoning classifier ([`crate::models::reasoning`]) needs a model's
 * embedded chat template (`tokenizer.chat_template`) and architecture
 * (`general.architecture`). Both live in the GGUF metadata key-value header at
 * the very start of the file, before any tensor data, so they can be read
 * straight off the downloaded blob with no engine load and no network.
 *
 * This reader extracts ONLY those two string values; every other value is
 * skipped by computing its on-disk size and seeking past it (the giant
 * tokenizer arrays are never materialized). It is deliberately forgiving: any
 * malformed, truncated, or hostile input resolves to "what was found so far"
 * (often `None`) rather than panicking, matching Thuki's never-panic-on-input
 * contract. A miss is harmless because the runtime behavioral backstop
 * self-corrects an `Always` model from its real output.
 *
 * Format reference: the GGUF header is `magic("GGUF") | version(u32) |
 * tensor_count(u64) | metadata_kv_count(u64)`, followed by `metadata_kv_count`
 * key-value pairs. A key is `len(u64) | bytes`; a value is `type(u32)` then a
 * type-dependent payload. Only versions 2 and 3 (u64 counts) are accepted; the
 * obsolete v1 layout (u32 counts) is rejected.
 */

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::config::defaults::{MAX_GGUF_KEY_BYTES, MAX_GGUF_KV_COUNT, MAX_GGUF_STRING_BYTES};

/// GGUF value type tag for a UTF-8 string (`len(u64) | bytes`).
const GGUF_TYPE_STRING: u32 = 8;
/// GGUF value type tag for an array (`elem_type(u32) | count(u64) | elements`).
const GGUF_TYPE_ARRAY: u32 = 9;

/// Metadata extracted from a GGUF header. Each field is `None` when the
/// model does not carry it (or the reader stopped before reaching it).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GgufMetadata {
    /// The embedded Jinja chat template (`tokenizer.chat_template`).
    pub chat_template: Option<String>,
    /// The model architecture (`general.architecture`, e.g. `qwen3`, `gpt-oss`).
    pub architecture: Option<String>,
    /// File role hint (`general.type`, e.g. `model`, `mmproj`, `adapter`).
    pub general_type: Option<String>,
}

/// Reads `general.architecture`, `general.type`, and `tokenizer.chat_template`
/// from a GGUF stream. Returns `None` only when the stream is not a GGUF the
/// reader understands (bad magic, unsupported version, or a header too short to
/// carry the counts); a stream that is a valid GGUF but is truncated or
/// malformed partway through returns `Some` with whatever was decoded before
/// the fault.
///
/// Generic over [`Read`] + [`Seek`] so it is driven by an in-memory
/// [`std::io::Cursor`] in tests and a [`std::io::BufReader`] over the blob
/// file in production.
pub fn read_gguf_metadata<R: Read + Seek>(r: &mut R) -> Option<GgufMetadata> {
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).ok()?;
    if &magic != b"GGUF" {
        return None;
    }
    let version = read_u32_le(r)?;
    if version != 2 && version != 3 {
        return None;
    }
    // tensor_count is not needed: the metadata KV block precedes the tensor
    // info, so we never have to walk the tensors to reach the template.
    let _tensor_count = read_u64_le(r)?;
    let kv_count = read_u64_le(r)?;

    let mut meta = GgufMetadata::default();
    // Clamp the loop so a corrupt `metadata_kv_count` cannot drive an
    // unbounded scan; real models sit far below the cap.
    let limit = kv_count.min(MAX_GGUF_KV_COUNT);
    for _ in 0..limit {
        // Past this point every read failure is treated as "end of usable
        // metadata": break and return what was decoded so far, never `?` (a
        // truncation after the template was read must not discard it).
        let Some(key_len) = read_u64_le(r) else { break };
        if key_len > MAX_GGUF_KEY_BYTES {
            break;
        }
        let mut key = vec![0u8; key_len as usize];
        if r.read_exact(&mut key).is_err() {
            break;
        }
        let Some(value_type) = read_u32_le(r) else {
            break;
        };

        if value_type == GGUF_TYPE_STRING && key == b"tokenizer.chat_template" {
            match read_string_value(r) {
                Some(s) => meta.chat_template = Some(s),
                None => break,
            }
        } else if value_type == GGUF_TYPE_STRING && key == b"general.architecture" {
            match read_string_value(r) {
                Some(s) => meta.architecture = Some(s),
                None => break,
            }
        } else if value_type == GGUF_TYPE_STRING && key == b"general.type" {
            match read_string_value(r) {
                Some(s) => meta.general_type = Some(s),
                None => break,
            }
        } else if skip_value(r, value_type).is_none() {
            break;
        }

        // Role-critical fields plus template: stop walking once we have enough
        // for both reasoning classification and primary-vs-projector gating.
        if meta.chat_template.is_some()
            && meta.architecture.is_some()
            && meta.general_type.is_some()
        {
            break;
        }
        // Projectors often lack a chat template; architecture alone (clip) or
        // architecture + type is enough to stop after a reasonable scan.
        if meta.architecture.as_deref() == Some("clip") && meta.general_type.is_some() {
            break;
        }
    }
    Some(meta)
}

/// Opens `path`, wraps it in a buffered reader, and extracts its GGUF
/// metadata. Returns `None` when the file cannot be opened or is not a
/// readable GGUF. Coverage-off: a thin filesystem wrapper around
/// [`read_gguf_metadata`], which carries all the tested parsing logic.
#[cfg_attr(coverage_nightly, coverage(off))]
pub fn read_gguf_metadata_from_file(path: &Path) -> Option<GgufMetadata> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    read_gguf_metadata(&mut reader)
}

/// Reads a little-endian `u32`, or `None` on a short read.
fn read_u32_le<R: Read>(r: &mut R) -> Option<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).ok()?;
    Some(u32::from_le_bytes(b))
}

/// Reads a little-endian `u64`, or `None` on a short read.
fn read_u64_le<R: Read>(r: &mut R) -> Option<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b).ok()?;
    Some(u64::from_le_bytes(b))
}

/// Reads a GGUF string value (`len(u64) | bytes`) the reader wants to keep.
/// Refuses a length above [`MAX_GGUF_STRING_BYTES`] so a corrupt length cannot
/// force a huge allocation. Decodes lossily so a non-UTF-8 byte never drops an
/// otherwise-usable template.
fn read_string_value<R: Read>(r: &mut R) -> Option<String> {
    let len = read_u64_le(r)?;
    if len > MAX_GGUF_STRING_BYTES {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// On-disk byte size of a fixed-width GGUF scalar value type, or `None` for a
/// non-scalar (string, array) or unknown type tag.
fn scalar_size(value_type: u32) -> Option<u64> {
    match value_type {
        // UINT8, INT8, BOOL
        0 | 1 | 7 => Some(1),
        // UINT16, INT16
        2 | 3 => Some(2),
        // UINT32, INT32, FLOAT32
        4..=6 => Some(4),
        // UINT64, INT64, FLOAT64
        10..=12 => Some(8),
        _ => None,
    }
}

/// Advances the stream past a value of `value_type` without materializing it.
/// Returns `None` on an unknown type, a malformed array, or a seek/read fault.
fn skip_value<R: Read + Seek>(r: &mut R, value_type: u32) -> Option<()> {
    match value_type {
        GGUF_TYPE_STRING => {
            let len = read_u64_le(r)?;
            seek_forward(r, len)
        }
        GGUF_TYPE_ARRAY => skip_array(r),
        scalar => {
            let n = scalar_size(scalar)?;
            seek_forward(r, n)
        }
    }
}

/// Skips an array value: `elem_type(u32) | count(u64) | elements`. A scalar
/// element array is skipped in one seek; a string element array is walked
/// element by element (each string is length-prefixed). Nested arrays and
/// unknown element types are unsupported and return `None`.
fn skip_array<R: Read + Seek>(r: &mut R) -> Option<()> {
    let elem_type = read_u32_le(r)?;
    let count = read_u64_le(r)?;
    match elem_type {
        GGUF_TYPE_STRING => {
            for _ in 0..count {
                let len = read_u64_le(r)?;
                seek_forward(r, len)?;
            }
            Some(())
        }
        GGUF_TYPE_ARRAY => None,
        scalar => {
            let size = scalar_size(scalar)?;
            let total = size.checked_mul(count)?;
            seek_forward(r, total)
        }
    }
}

/// Seeks `n` bytes forward from the current position. Refuses a `n` that does
/// not fit in the seek offset type so a corrupt length cannot wrap.
fn seek_forward<R: Seek>(r: &mut R, n: u64) -> Option<()> {
    let offset = i64::try_from(n).ok()?;
    r.seek(SeekFrom::Current(offset)).ok()?;
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // ── GGUF byte builders (mirror the on-disk layout) ───────────────────────

    /// Encodes a GGUF string: `len(u64) | bytes`.
    fn enc_string(s: &[u8]) -> Vec<u8> {
        let mut v = (s.len() as u64).to_le_bytes().to_vec();
        v.extend_from_slice(s);
        v
    }

    /// Encodes a string-valued KV pair: `key | type(8) | value`.
    fn kv_string(key: &str, value: &[u8]) -> Vec<u8> {
        let mut v = enc_string(key.as_bytes());
        v.extend_from_slice(&GGUF_TYPE_STRING.to_le_bytes());
        v.extend_from_slice(&enc_string(value));
        v
    }

    /// Encodes a scalar KV pair with a raw `value_type` and raw payload bytes.
    fn kv_scalar(key: &str, value_type: u32, payload: &[u8]) -> Vec<u8> {
        let mut v = enc_string(key.as_bytes());
        v.extend_from_slice(&value_type.to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    /// Encodes a `key | type(9) | elem_type | count | elements` array KV.
    fn kv_array(key: &str, elem_type: u32, count: u64, elements: &[u8]) -> Vec<u8> {
        let mut v = enc_string(key.as_bytes());
        v.extend_from_slice(&GGUF_TYPE_ARRAY.to_le_bytes());
        v.extend_from_slice(&elem_type.to_le_bytes());
        v.extend_from_slice(&count.to_le_bytes());
        v.extend_from_slice(elements);
        v
    }

    /// Assembles a full GGUF header from `version` and pre-encoded KV blobs.
    fn build_gguf(version: u32, kvs: &[Vec<u8>]) -> Vec<u8> {
        let mut v = b"GGUF".to_vec();
        v.extend_from_slice(&version.to_le_bytes());
        v.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        v.extend_from_slice(&(kvs.len() as u64).to_le_bytes()); // metadata_kv_count
        for kv in kvs {
            v.extend_from_slice(kv);
        }
        v
    }

    fn read(bytes: &[u8]) -> Option<GgufMetadata> {
        read_gguf_metadata(&mut Cursor::new(bytes.to_vec()))
    }

    // ── Happy paths ──────────────────────────────────────────────────────────

    #[test]
    fn extracts_template_and_architecture() {
        let bytes = build_gguf(
            3,
            &[
                kv_string("general.architecture", b"qwen3"),
                kv_string("tokenizer.chat_template", b"{%- if enable_thinking %}"),
            ],
        );
        let meta = read(&bytes).unwrap();
        assert_eq!(meta.architecture.as_deref(), Some("qwen3"));
        assert_eq!(
            meta.chat_template.as_deref(),
            Some("{%- if enable_thinking %}")
        );
    }

    #[test]
    fn version_2_is_accepted() {
        let bytes = build_gguf(2, &[kv_string("tokenizer.chat_template", b"<think>")]);
        let meta = read(&bytes).unwrap();
        assert_eq!(meta.chat_template.as_deref(), Some("<think>"));
    }

    #[test]
    fn skips_scalar_kv_before_target() {
        let bytes = build_gguf(
            3,
            &[
                kv_scalar("some.u16", 2, &7u16.to_le_bytes()),
                kv_scalar("some.i16", 3, &(-3i16).to_le_bytes()),
                kv_scalar("some.u32", 4, &7u32.to_le_bytes()),
                kv_scalar("some.bool", 7, &[1]),
                kv_scalar("some.f64", 12, &1.5f64.to_le_bytes()),
                kv_string("tokenizer.chat_template", b"<|channel|>"),
            ],
        );
        assert_eq!(
            read(&bytes).unwrap().chat_template.as_deref(),
            Some("<|channel|>")
        );
    }

    #[test]
    fn skips_scalar_array_before_target() {
        // token_type-style INT32 array: 3 elements, skipped in one seek.
        let elems: Vec<u8> = [1i32, 2, 3].iter().flat_map(|n| n.to_le_bytes()).collect();
        let bytes = build_gguf(
            3,
            &[
                kv_array("tokenizer.ggml.token_type", 5, 3, &elems),
                kv_string("tokenizer.chat_template", b"<think>"),
            ],
        );
        assert_eq!(
            read(&bytes).unwrap().chat_template.as_deref(),
            Some("<think>")
        );
    }

    #[test]
    fn skips_string_array_before_target() {
        // tokens-style string array walked element by element.
        let mut elems = Vec::new();
        elems.extend_from_slice(&enc_string(b"a"));
        elems.extend_from_slice(&enc_string(b"bb"));
        let bytes = build_gguf(
            3,
            &[
                kv_array("tokenizer.ggml.tokens", GGUF_TYPE_STRING, 2, &elems),
                kv_string("tokenizer.chat_template", b"<thought>"),
            ],
        );
        assert_eq!(
            read(&bytes).unwrap().chat_template.as_deref(),
            Some("<thought>")
        );
    }

    #[test]
    fn architecture_only_is_returned_without_template() {
        let bytes = build_gguf(3, &[kv_string("general.architecture", b"gemma3")]);
        let meta = read(&bytes).unwrap();
        assert_eq!(meta.architecture.as_deref(), Some("gemma3"));
        assert_eq!(meta.chat_template, None);
    }

    #[test]
    fn extracts_general_type() {
        let bytes = build_gguf(
            3,
            &[
                kv_string("general.architecture", b"clip"),
                kv_string("general.type", b"mmproj"),
            ],
        );
        let meta = read(&bytes).unwrap();
        assert_eq!(meta.architecture.as_deref(), Some("clip"));
        assert_eq!(meta.general_type.as_deref(), Some("mmproj"));
    }

    #[test]
    fn stops_after_template_arch_and_type_ignoring_trailing_malformed() {
        let bad_nested = kv_array("trailing.bad", GGUF_TYPE_ARRAY, 1, &[]);
        let bytes = build_gguf(
            3,
            &[
                kv_string("general.architecture", b"qwen3"),
                kv_string("general.type", b"model"),
                kv_string("tokenizer.chat_template", b"<think>"),
                bad_nested,
            ],
        );
        let meta = read(&bytes).unwrap();
        assert_eq!(meta.architecture.as_deref(), Some("qwen3"));
        assert_eq!(meta.general_type.as_deref(), Some("model"));
        assert_eq!(meta.chat_template.as_deref(), Some("<think>"));
    }

    #[test]
    fn general_type_string_too_large_stops_scan() {
        let mut kv = enc_string(b"general.type");
        kv.extend_from_slice(&GGUF_TYPE_STRING.to_le_bytes());
        kv.extend_from_slice(&(MAX_GGUF_STRING_BYTES + 1).to_le_bytes());
        let bytes = build_gguf(3, &[kv]);
        assert_eq!(read(&bytes), Some(GgufMetadata::default()));
    }

    #[test]
    fn stops_after_both_found_ignoring_trailing_malformed_kv() {
        // A nested-array KV (unsupported) AFTER both targets must not matter:
        // the early-exit returns before the reader reaches it.
        let bad_nested = kv_array("trailing.bad", GGUF_TYPE_ARRAY, 1, &[]);
        let bytes = build_gguf(
            3,
            &[
                kv_string("general.architecture", b"qwen3"),
                kv_string("tokenizer.chat_template", b"<think>"),
                bad_nested,
            ],
        );
        let meta = read(&bytes).unwrap();
        assert_eq!(meta.architecture.as_deref(), Some("qwen3"));
        assert_eq!(meta.chat_template.as_deref(), Some("<think>"));
    }

    #[test]
    fn lossy_decode_keeps_non_utf8_template() {
        // An invalid UTF-8 byte (0xff) is replaced, not dropped.
        let bytes = build_gguf(3, &[kv_string("tokenizer.chat_template", b"<think>\xff")]);
        let template = read(&bytes).unwrap().chat_template.unwrap();
        assert!(template.starts_with("<think>"));
    }

    // ── Header rejections (return None) ──────────────────────────────────────

    #[test]
    fn bad_magic_is_none() {
        assert_eq!(read(b"NOPExxxxxxxxxxxxxxxxxxxx"), None);
    }

    #[test]
    fn unsupported_version_is_none() {
        let bytes = build_gguf(1, &[kv_string("tokenizer.chat_template", b"<think>")]);
        assert_eq!(read(&bytes), None);
    }

    #[test]
    fn truncated_before_counts_is_none() {
        // "GGUF" + version only, no tensor/kv counts.
        let mut bytes = b"GGUF".to_vec();
        bytes.extend_from_slice(&3u32.to_le_bytes());
        assert_eq!(read(&bytes), None);
    }

    // ── Mid-scan faults (return partial Some) ────────────────────────────────

    #[test]
    fn claimed_kv_but_no_body_returns_empty() {
        // metadata_kv_count says 1 but the stream ends right after the counts.
        let mut bytes = b"GGUF".to_vec();
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes()); // tensor_count
        bytes.extend_from_slice(&1u64.to_le_bytes()); // kv_count = 1, but no KV follows
        assert_eq!(read(&bytes), Some(GgufMetadata::default()));
    }

    #[test]
    fn oversized_key_length_stops_scan() {
        let mut huge_key = (MAX_GGUF_KEY_BYTES + 1).to_le_bytes().to_vec();
        huge_key.extend_from_slice(&0u32.to_le_bytes()); // a stray type, never reached
        let bytes = build_gguf(3, &[huge_key]);
        assert_eq!(read(&bytes), Some(GgufMetadata::default()));
    }

    #[test]
    fn truncated_key_bytes_stops_scan() {
        // key_len claims 10 bytes but only 2 follow.
        let mut kv = 10u64.to_le_bytes().to_vec();
        kv.extend_from_slice(b"ab");
        let bytes = build_gguf(3, &[kv]);
        assert_eq!(read(&bytes), Some(GgufMetadata::default()));
    }

    #[test]
    fn truncated_before_value_type_stops_scan() {
        // A complete key but the stream ends before the value type u32.
        let kv = enc_string(b"general.architecture");
        let bytes = build_gguf(3, &[kv]);
        assert_eq!(read(&bytes), Some(GgufMetadata::default()));
    }

    #[test]
    fn target_string_value_too_large_stops_scan() {
        let mut kv = enc_string(b"tokenizer.chat_template");
        kv.extend_from_slice(&GGUF_TYPE_STRING.to_le_bytes());
        kv.extend_from_slice(&(MAX_GGUF_STRING_BYTES + 1).to_le_bytes());
        let bytes = build_gguf(3, &[kv]);
        assert_eq!(read(&bytes), Some(GgufMetadata::default()));
    }

    #[test]
    fn target_string_value_truncated_stops_scan() {
        // Architecture value claims 20 bytes but only 3 are present.
        let mut kv = enc_string(b"general.architecture");
        kv.extend_from_slice(&GGUF_TYPE_STRING.to_le_bytes());
        kv.extend_from_slice(&20u64.to_le_bytes());
        kv.extend_from_slice(b"abc");
        let bytes = build_gguf(3, &[kv]);
        assert_eq!(read(&bytes), Some(GgufMetadata::default()));
    }

    #[test]
    fn unknown_value_type_stops_scan() {
        // Value type 99 is not a real GGUF type: the skip fails and the scan
        // stops, but a target read before it is still returned.
        let bytes = build_gguf(
            3,
            &[
                kv_string("tokenizer.chat_template", b"<think>"),
                kv_scalar("weird", 99, &[0, 0, 0, 0]),
            ],
        );
        // chat_template was read first, then early-exit never triggers (arch
        // missing) so the unknown type is reached and stops the scan; the
        // template is preserved.
        let meta = read(&bytes).unwrap();
        assert_eq!(meta.chat_template.as_deref(), Some("<think>"));
    }

    #[test]
    fn nested_array_element_stops_scan() {
        // An array whose elements are themselves arrays is unsupported.
        let bytes = build_gguf(3, &[kv_array("bad.nested", GGUF_TYPE_ARRAY, 1, &[])]);
        assert_eq!(read(&bytes), Some(GgufMetadata::default()));
    }

    #[test]
    fn array_count_overflow_stops_scan() {
        // count * elem_size overflows u64; the checked multiply bails.
        let mut kv = enc_string(b"bad.overflow");
        kv.extend_from_slice(&GGUF_TYPE_ARRAY.to_le_bytes());
        kv.extend_from_slice(&12u32.to_le_bytes()); // FLOAT64, size 8
        kv.extend_from_slice(&u64::MAX.to_le_bytes()); // count
        let bytes = build_gguf(3, &[kv]);
        assert_eq!(read(&bytes), Some(GgufMetadata::default()));
    }

    #[test]
    fn skip_string_value_advances_to_next_kv() {
        // A non-target string KV is skipped (not kept), then the target read.
        let bytes = build_gguf(
            3,
            &[
                kv_string("general.name", b"Some Model"),
                kv_string("tokenizer.chat_template", b"<seed:think>"),
            ],
        );
        let meta = read(&bytes).unwrap();
        assert_eq!(meta.chat_template.as_deref(), Some("<seed:think>"));
    }

    #[test]
    fn file_wrapper_reads_a_written_gguf() {
        let dir = std::env::temp_dir().join(format!("thuki-gguf-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("model.gguf");
        let bytes = build_gguf(3, &[kv_string("tokenizer.chat_template", b"<|channel|>")]);
        std::fs::write(&path, &bytes).unwrap();

        let meta = read_gguf_metadata_from_file(&path).unwrap();
        assert_eq!(meta.chat_template.as_deref(), Some("<|channel|>"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_wrapper_missing_file_is_none() {
        let path = std::env::temp_dir().join("thuki-gguf-does-not-exist.gguf");
        assert_eq!(read_gguf_metadata_from_file(&path), None);
    }
}
