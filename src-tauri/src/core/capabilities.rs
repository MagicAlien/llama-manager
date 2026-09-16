//! T-037 — model capability tags.
//!
//! What a model can actually do, decided from the model itself and nothing
//! else: the GGUF header for Thinking / MTP / Tool use, the model's own file
//! set for Vision. No filename is ever consulted, and no capability is
//! inferred from an architecture whitelist — a file whose name announces a
//! capability it does not have, and one whose name hides a capability it does
//! have, both classify by their contents (`docs/TASKS.md` T-037).
//!
//! Pure: takes already-parsed values, does no I/O, reads no database (the
//! rule `AGENTS.md` invariant 5 states for the estimator, applied here to a
//! much smaller classifier).

use super::types::{CapabilityTag, GgufMetadata, LaunchParams};

/// The tags a model earns, in the order the UI renders them.
///
/// A model with none of the four returns an empty vector — the screens render
/// no tag row at all for that case, rather than an empty row or a set of
/// placeholders.
pub fn tags_for(metadata: &GgufMetadata, launch_params: &LaunchParams) -> Vec<CapabilityTag> {
    let mut tags = Vec::new();

    if metadata.supports_thinking {
        tags.push(CapabilityTag::Thinking);
    }
    if metadata.has_mtp_heads {
        tags.push(CapabilityTag::Mtp);
    }
    // Vision is not a property of the model file alone: it is the presence of
    // the projector in the model's own file set, which T-031's directory-based
    // association resolved into `mmproj_path` at import time. A projector is
    // never a launchable catalogue entry of its own, so "has vision" and "has
    // a projector" are the same statement.
    if launch_params.mmproj_path.is_some() {
        tags.push(CapabilityTag::Vision);
    }
    if metadata.supports_tools {
        tags.push(CapabilityTag::ToolUse);
    }

    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A metadata value speaking for a model that declares nothing.
    fn plain_metadata() -> GgufMetadata {
        GgufMetadata {
            architecture: "llama".to_string(),
            param_count: Some(8_000_000_000),
            quantization: "Q4_K_M".to_string(),
            block_count: 32,
            context_length: Some(8192),
            embedding_length: Some(4096),
            attention_head_count: Some(32),
            attention_head_count_kv: Some(8),
            has_chat_template: true,
            is_moe: false,
            expert_count: None,
            is_draft_model: false,
            has_mtp_heads: false,
            supports_tools: false,
            supports_thinking: false,
        }
    }

    fn empty_launch_params() -> LaunchParams {
        LaunchParams::default()
    }

    #[test]
    fn a_model_with_no_capability_earns_no_tags() {
        let tags = tags_for(&plain_metadata(), &empty_launch_params());
        assert!(
            tags.is_empty(),
            "a model that declares nothing must produce no tags, got {tags:?}"
        );
    }

    #[test]
    fn every_capability_earns_exactly_its_own_tag() {
        let mut metadata = plain_metadata();
        metadata.supports_thinking = true;
        metadata.has_mtp_heads = true;
        metadata.supports_tools = true;
        let mut params = empty_launch_params();
        params.mmproj_path = Some("C:/models/mmproj-F16.gguf".into());

        assert_eq!(
            tags_for(&metadata, &params),
            vec![
                CapabilityTag::Thinking,
                CapabilityTag::Mtp,
                CapabilityTag::Vision,
                CapabilityTag::ToolUse,
            ],
            "the tag order is the one the UI renders and the one the task lists"
        );
    }

    #[test]
    fn each_signal_on_its_own_earns_its_own_tag_and_no_other() {
        let mut only_thinking = plain_metadata();
        only_thinking.supports_thinking = true;
        assert_eq!(
            tags_for(&only_thinking, &empty_launch_params()),
            vec![CapabilityTag::Thinking]
        );

        let mut only_mtp = plain_metadata();
        only_mtp.has_mtp_heads = true;
        assert_eq!(
            tags_for(&only_mtp, &empty_launch_params()),
            vec![CapabilityTag::Mtp]
        );

        let mut only_tools = plain_metadata();
        only_tools.supports_tools = true;
        assert_eq!(
            tags_for(&only_tools, &empty_launch_params()),
            vec![CapabilityTag::ToolUse]
        );

        let mut params_with_projector = empty_launch_params();
        params_with_projector.mmproj_path = Some("C:/models/mmproj-F16.gguf".into());
        assert_eq!(
            tags_for(&plain_metadata(), &params_with_projector),
            vec![CapabilityTag::Vision]
        );
    }

    #[test]
    fn thinking_and_tool_use_are_independent_of_each_other() {
        // The two questions have different answers in the same file, so one
        // must never imply the other.
        let mut tools_only = plain_metadata();
        tools_only.supports_tools = true;
        assert_eq!(
            tags_for(&tools_only, &empty_launch_params()),
            vec![CapabilityTag::ToolUse]
        );

        let mut thinking_only = plain_metadata();
        thinking_only.supports_thinking = true;
        assert_eq!(
            tags_for(&thinking_only, &empty_launch_params()),
            vec![CapabilityTag::Thinking]
        );
    }
}
