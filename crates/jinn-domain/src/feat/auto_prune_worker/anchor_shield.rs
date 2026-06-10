//! Anchor-shield auto-prune worker.
//!
//! Emits [`ContextOverride::ForcedInclude`] for all in-context-by-default entry types
//! (`User`, `Assistant`, `ToolCall`, `ToolResult`) within a configurable radius of any
//! anchor entry. This prevents other auto-prune workers from excluding entries that carry
//! conversation structure near user turns.
//!
//! # Anchors
//!
//! An entry is an anchor if any of:
//! - It is a `User` entry (any position).
//! - It is the **first** entry in history (regardless of type).
//! - It is the **last** entry in history (regardless of type).
//!
//! The anchor definition is shared with [`AnchoredAssistantAutoPruneWorker`] via
//! [`collect_anchor_indices`].
//!
//! # Pair Atomicity
//!
//! `ToolCall` and `ToolResult` entries form pairs. If either half is within the shield
//! radius, the other half is also shielded — regardless of its own distance to the
//! nearest anchor. This prevents orphaned tool calls or results in LLM context.
//!
//! # Relationship to Anchored-Assistant Prune Worker
//!
//! The shield worker and the [`AnchoredAssistantAutoPruneWorker`] share a single
//! `radius` value (configured on [`AnchorShieldConfig`]). The shield protects everything
//! within the radius; the prune worker removes everything beyond it. They partition
//! cleanly because:
//! - Shield: distance ≤ radius → `ForcedInclude`
//! - Prune: distance > radius → `ForcedExclude`
//!
//! [`ContextOverride::ForcedInclude`]: crate::feat::session::chat_entry::ContextOverride::ForcedInclude
//! [`AnchoredAssistantAutoPruneWorker`]: super::AnchoredAssistantAutoPruneWorker
//! [`collect_anchor_indices`]: super::anchored_assistant::collect_anchor_indices
//! [`AnchorShieldConfig`]: crate::feat::preferences_actor::user_preferences::AnchorShieldConfig

/// Placeholder — full implementation in Phase 2.
#[derive(Clone)]
pub struct AnchorShieldAutoPruneWorker;
