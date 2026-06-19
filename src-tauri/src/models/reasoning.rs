/*!
 * Dynamic reasoning-capability classifier for locally-run GGUF models.
 *
 * Thuki must behave correctly for ANY model a user downloads, not just the
 * three curated starters whose class is baked into the registry. The single
 * authoritative signal a GGUF carries about whether (and how) it reasons is
 * its embedded chat template ([`tokenizer.chat_template`]); the template's
 * markers tell us which reasoning family a model belongs to.
 *
 * This module is the pure, side-effect-free heart of the classifier:
 * [`classify_reasoning`] maps a chat-template string (plus the optional
 * `general.architecture`) onto one of three classes. The byte-level template
 * extraction lives in [`crate::models::gguf`]; persistence and the runtime
 * behavioral backstop live in [`crate::models`] / [`crate::commands`].
 *
 * The three classes mirror the convergent industry taxonomy (OpenRouter
 * `mandatory`, Ollama `thinking` capability, vLLM per-family parsers):
 *
 * - [`ReasoningClass::None`] — not a reasoning model. `/think` is a no-op, no
 *   thinking block, no badge.
 * - [`ReasoningClass::Optional`] — reasoning can be turned off. Thuki defaults
 *   it OFF (the OFF blast in [`crate::openai`] suppresses it) and `/think`
 *   turns it on per-message. No badge.
 * - [`ReasoningClass::Always`] — reasoning is structural and cannot be turned
 *   off. Thuki shows it cleanly and badges the model so the latency is not a
 *   surprise; `/think` is a harmless no-op.
 */

/// Marker present in gpt-oss / Harmony templates: reasoning rides the
/// `analysis` channel, which is structural and cannot be disabled.
const MARKER_HARMONY_CHANNEL: &str = "<|channel|>";

/// GGUF `general.architecture` value for gpt-oss / Harmony models. Used as a
/// belt-and-suspenders signal alongside [`MARKER_HARMONY_CHANNEL`] so the
/// curated Smartest starter (gpt-oss) classifies as `Always` even if a GGUF
/// variant lays its channel markup out differently than expected.
const ARCH_GPT_OSS: &str = "gpt-oss";

/// The literal word that every "reasoning can be disabled" family threads
/// through its template, whether as a kwarg (`enable_thinking`,
/// `thinking_budget`) or a bare Jinja variable (`thinking`). Crucially the
/// always-on tag families spell their tags `<think>` / `<thought>` /
/// `<seed:think>` (no `ing`), so the presence of the whole word `thinking`
/// is what separates "has an off switch" from "always reasons".
const MARKER_THINKING_KWARG: &str = "thinking";

/// Mistral Magistral / Ministral reasoning tags. Reasoning is driven by a
/// system-prompt instruction rather than a template kwarg, so without Thuki's
/// (absent) reasoning system prompt these models stay quiet: treated as
/// `Optional` (default off), not `Always`.
const MARKER_MISTRAL_THINK_OPEN: &str = "[THINK]";
const MARKER_MISTRAL_THINK_CLOSE: &str = "[/THINK]";

/// Always-on reasoning tags: a template that hard-opens one of these on the
/// assistant turn and offers no off switch always reasons (DeepSeek-R1 and
/// distills, QwQ, EXAONE-Deep, MiniMax-M2, Phi-4-reasoning, Seed-OSS variants
/// without a budget kwarg). Checked only AFTER the off-switch word, so a
/// family that ships both a tag and a kwarg (e.g. Seed-OSS `<seed:think>` +
/// `thinking_budget`) is correctly classified `Optional`.
const ALWAYS_TAGS: &[&str] = &[
    "<think>",
    "</think>",
    "<thought>",
    "</thought>",
    "<seed:think>",
];

/// How a model reasons, derived from its chat template. See the module docs
/// for the behavior each class drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningClass {
    /// Not a reasoning model.
    None,
    /// Reasoning can be turned off; Thuki defaults it off.
    Optional,
    /// Reasoning is structural and cannot be turned off.
    Always,
}

impl ReasoningClass {
    /// Projects the class onto the two manifest capability flags Thuki
    /// persists and surfaces: `(thinking, reasoning_always)`.
    ///
    /// - `None`     -> `(false, false)`: no thinking block, no badge.
    /// - `Optional` -> `(true,  false)`: thinking available, no badge.
    /// - `Always`   -> `(true,  true )`: thinking shown, badge.
    pub fn flags(self) -> (bool, bool) {
        match self {
            ReasoningClass::None => (false, false),
            ReasoningClass::Optional => (true, false),
            ReasoningClass::Always => (true, true),
        }
    }
}

/// Classifies a model's reasoning capability from its chat template and
/// optional `general.architecture`, applying the family markers most-specific
/// first:
///
/// 1. gpt-oss / Harmony (`<|channel|>` or `gpt-oss` architecture) -> `Always`.
/// 2. An off-switch word (`enable_thinking` / `thinking` / `thinking_budget`)
///    anywhere in the template -> `Optional` (the OFF blast controls it).
/// 3. Mistral `[THINK]` / `[/THINK]` tags -> `Optional` (system-prompt
///    driven; quiet without Thuki's reasoning prompt).
/// 4. An always-on reasoning tag (`<think>` / `<thought>` / `<seed:think>`)
///    with no off switch -> `Always`.
/// 5. No reasoning markers at all -> `None`.
///
/// Never panics: any input (empty, binary garbage decoded as text, a template
/// from a future family) resolves to one of the three classes. When the
/// template scan is wrong for an `Always` model, the runtime behavioral
/// backstop self-corrects from real output, so this fast path only needs to
/// be right for the common families.
pub fn classify_reasoning(chat_template: &str, architecture: Option<&str>) -> ReasoningClass {
    let arch_is_gpt_oss = architecture
        .map(|a| {
            let lower = a.to_ascii_lowercase();
            lower.contains(ARCH_GPT_OSS) || lower.contains("gptoss")
        })
        .unwrap_or(false);

    // 1. gpt-oss / Harmony: highest-signal, structural reasoning channel.
    if chat_template.contains(MARKER_HARMONY_CHANNEL) || arch_is_gpt_oss {
        return ReasoningClass::Always;
    }

    // 2. Any "off switch" word means the model reads a disable signal and the
    //    OFF blast already controls it. Covers `enable_thinking`,
    //    `thinking_budget`, and a bare `thinking` Jinja variable in one check,
    //    because the always-on tag families never spell the whole word.
    if chat_template.contains(MARKER_THINKING_KWARG) {
        return ReasoningClass::Optional;
    }

    // 3. Mistral reasoning is system-prompt driven, not template-gated, so it
    //    is quiet by default under Thuki and treated as optional.
    if chat_template.contains(MARKER_MISTRAL_THINK_OPEN)
        || chat_template.contains(MARKER_MISTRAL_THINK_CLOSE)
    {
        return ReasoningClass::Optional;
    }

    // 4. A reasoning tag with no off switch: the model always reasons.
    if ALWAYS_TAGS.iter().any(|tag| chat_template.contains(tag)) {
        return ReasoningClass::Always;
    }

    // 5. No markers: not a reasoning model.
    ReasoningClass::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_map_each_class() {
        assert_eq!(ReasoningClass::None.flags(), (false, false));
        assert_eq!(ReasoningClass::Optional.flags(), (true, false));
        assert_eq!(ReasoningClass::Always.flags(), (true, true));
    }

    // ── Always: gpt-oss / Harmony ────────────────────────────────────────────

    #[test]
    fn gpt_oss_channel_marker_is_always() {
        let t = "<|start|>system<|message|>...<|channel|>analysis<|message|>...";
        assert_eq!(classify_reasoning(t, None), ReasoningClass::Always);
    }

    #[test]
    fn gpt_oss_architecture_is_always_even_without_channel_marker() {
        // A gpt-oss GGUF whose template the scan does not recognize still
        // classifies Always from the architecture tiebreak.
        assert_eq!(
            classify_reasoning("{{ messages }}", Some("gpt-oss")),
            ReasoningClass::Always
        );
        assert_eq!(
            classify_reasoning("", Some("GptOss")),
            ReasoningClass::Always
        );
    }

    // ── Always: tag families with no off switch ──────────────────────────────

    #[test]
    fn deepseek_r1_hard_open_think_is_always() {
        // R1 hard-opens <think> after the assistant marker and reads no kwarg.
        let t = "{{'<｜Assistant｜>'}}<think>\\n";
        assert_eq!(classify_reasoning(t, None), ReasoningClass::Always);
    }

    #[test]
    fn qwq_think_tag_qwen2_is_always() {
        let t = "<|im_start|>assistant\\n<think>\\n";
        assert_eq!(classify_reasoning(t, Some("qwen2")), ReasoningClass::Always);
    }

    #[test]
    fn exaone_deep_thought_tag_is_always() {
        let t = "<|assistant|>\\n<thought>";
        assert_eq!(classify_reasoning(t, None), ReasoningClass::Always);
    }

    #[test]
    fn closing_think_tag_alone_is_always() {
        // Some templates only carry the closing tag in a prefill branch.
        assert_eq!(
            classify_reasoning("...</think>...", None),
            ReasoningClass::Always
        );
        assert_eq!(
            classify_reasoning("...</thought>...", None),
            ReasoningClass::Always
        );
    }

    // ── Optional: off-switch kwarg / variable families ───────────────────────

    #[test]
    fn qwen3_enable_thinking_is_optional() {
        let t = "{%- if enable_thinking %}<think>{% endif %}";
        assert_eq!(
            classify_reasoning(t, Some("qwen3")),
            ReasoningClass::Optional
        );
    }

    #[test]
    fn glm_enable_thinking_is_optional() {
        let t = "<|assistant|>{% if enable_thinking %}...{% endif %}";
        assert_eq!(classify_reasoning(t, None), ReasoningClass::Optional);
    }

    #[test]
    fn granite_thinking_variable_is_optional() {
        let t = "<|start_of_role|>{% if thinking %}...{% endif %}";
        assert_eq!(classify_reasoning(t, None), ReasoningClass::Optional);
    }

    #[test]
    fn deepseek_v31_thinking_branch_is_optional() {
        let t = "{{'<｜Assistant｜>'}}{% if thinking %}<think>{% else %}</think>{% endif %}";
        assert_eq!(classify_reasoning(t, None), ReasoningClass::Optional);
    }

    #[test]
    fn seed_oss_budget_kwarg_wins_over_its_tag() {
        // Seed-OSS ships both <seed:think> AND thinking_budget; the budget
        // (off switch) must win so it is Optional, not Always.
        let t = "<seed:think>{{ thinking_budget }}";
        assert_eq!(classify_reasoning(t, None), ReasoningClass::Optional);
    }

    #[test]
    fn mistral_bracket_think_is_optional() {
        // Magistral reasoning is system-prompt driven; quiet by default.
        assert_eq!(
            classify_reasoning("...[THINK]...", None),
            ReasoningClass::Optional
        );
        assert_eq!(
            classify_reasoning("...[/THINK]...", None),
            ReasoningClass::Optional
        );
    }

    // ── None: plain instruct models ──────────────────────────────────────────

    #[test]
    fn gemma_plain_instruct_is_none() {
        let t = "<start_of_turn>user\\n{{ content }}<end_of_turn>";
        assert_eq!(classify_reasoning(t, Some("gemma3")), ReasoningClass::None);
    }

    #[test]
    fn empty_template_is_none() {
        assert_eq!(classify_reasoning("", None), ReasoningClass::None);
    }

    #[test]
    fn arch_without_markers_does_not_force_a_class() {
        // A non-gpt-oss architecture with no template markers stays None: the
        // architecture only tiebreaks the gpt-oss case.
        assert_eq!(
            classify_reasoning("{{ messages }}", Some("llama")),
            ReasoningClass::None
        );
    }

    #[test]
    fn channel_marker_beats_a_later_thinking_word() {
        // Ordering guard: a Harmony template that also happens to mention the
        // word "thinking" still classifies Always (channel checked first).
        let t = "<|channel|>analysis ... enable_thinking";
        assert_eq!(classify_reasoning(t, None), ReasoningClass::Always);
    }
}
