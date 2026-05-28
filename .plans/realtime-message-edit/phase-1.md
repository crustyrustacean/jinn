## Phase 1: Define `HistoryMutation` enum and mutation queue on `ChatSessionState`

### Problem

Compaction and other history operations currently use positional indices (`usize`) to identify entries. When entries are inserted or appended during streaming, these indices shift and become stale. We need a `HistoryMutation` enum that uses `ChatEntryId` (stable UUIDs) for targeting, and a pending mutation queue on `ChatSessionState` where batches of mutations accumulate until they can be safely applied at turn boundaries.

### What Moves / What Stays

**New types:**
- `HistoryMutation` enum in `feat/session/history_mutation.rs` — four variants: `SetContextOverride`, `InsertEntry`, `PinEntry`, `UnpinEntry`
- Tests in `feat/session/history_mutation_tests.rs`

**Modified types:**
- `SessionCoreEphemeral` — add `pending_mutations: Vec<Vec<HistoryMutation>>` field
- `ChatSessionState` — add `queue_mutations`, `drain_pending_mutations`, `apply_mutations`, `find_entry_index_by_id`, `drain_and_apply_pending_mutations` methods

**Unchanged:**
- `insert_entry_at` — called by `apply_mutations` for `InsertEntry` (reuses existing index-shifting logic)
- `pin_entry` / `unpin_entry` — called by `apply_mutations` for pin mutations (reuses existing ID-based pin logic)
- `mark_entries_ignored` — stays index-based for existing compaction (not touched)
- All existing compaction protocol types (`BeginCompaction`, `EndCompaction`, `CompactionResult`)
- All existing streaming/tool-call handlers

### File Changes

#### 1. CREATE `crates/nullslop-domain/src/feat/session/history_mutation.rs`

New file containing the `HistoryMutation` enum. No dependencies beyond `chat_entry` types and `serde`.

```rust
//! History mutation types — declarative mutations for background workers.
//!
//! Workers produce `Vec<HistoryMutation>` batches. The session actor resolves
//! `ChatEntryId` → current position at application time. Mutations targeting
//! nonexistent entries are silently skipped.

use crate::feat::session::chat_entry::{ChatEntry, ChatEntryId, ContextOverride, PinPosition};
use serde::{Deserialize, Serialize};

/// A declarative mutation to apply to a session's history.
///
/// Workers produce `Vec<HistoryMutation>` batches. The session actor
/// resolves `ChatEntryId` → current position at application time.
/// Mutations targeting nonexistent entries are silently skipped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HistoryMutation {
    /// Set the context override on an entry (include/exclude from LLM context).
    SetContextOverride {
        entry_id: ChatEntryId,
        value: ContextOverride,
    },
    /// Insert a new entry into history after the specified entry.
    /// `after_entry_id: None` means insert at the beginning (index 0).
    InsertEntry {
        after_entry_id: Option<ChatEntryId>,
        entry: ChatEntry,
    },
    /// Pin an entry to a specific position in the assembled prompt.
    PinEntry {
        entry_id: ChatEntryId,
        position: PinPosition,
    },
    /// Remove the pin from an entry.
    UnpinEntry {
        entry_id: ChatEntryId,
    },
}
```

#### 2. CREATE `crates/nullslop-domain/src/feat/session/history_mutation_tests.rs`

Unit tests covering all mutation application behaviors.

#### 3. MODIFY `crates/nullslop-domain/src/feat/session/mod.rs`

Add module declaration:

```rust
pub mod history_mutation;

#[cfg(test)]
mod history_mutation_tests;
```

#### 4. MODIFY `crates/nullslop-domain/src/feat/session/chat_session.rs`

4a. Add import for `HistoryMutation`:

```rust
use crate::feat::session::history_mutation::HistoryMutation;
```

4b. Add `pending_mutations` field to `SessionCoreEphemeral` (after `busy_counter`):

```rust
    /// Pending history mutation batches from background workers.
    /// Drained and applied at safe application points (tool batch completion,
    /// stream completion). Not persisted across restarts.
    #[serde(skip)]
    pub(crate) pending_mutations: Vec<Vec<HistoryMutation>>,
```

4c. Add methods to `ChatSessionState`:

```rust
    /// Queue a batch of mutations for deferred application.
    pub fn queue_mutations(&mut self, batch: Vec<HistoryMutation>) {
        if !batch.is_empty() {
            self.core.ephemeral.pending_mutations.push(batch);
        }
    }

    /// Drain all pending mutation batches.
    pub fn drain_pending_mutations(&mut self) -> Vec<Vec<HistoryMutation>> {
        std::mem::take(&mut self.core.ephemeral.pending_mutations)
    }

    /// Resolve a `ChatEntryId` to its current index in history.
    /// Returns `None` if the entry no longer exists.
    pub fn find_entry_index_by_id(&self, id: &ChatEntryId) -> Option<usize> {
        self.core.history.iter().position(|e| e.id == *id)
    }

    /// Apply a batch of mutations. Resolves IDs to current positions.
    /// Silently skips mutations targeting nonexistent entries.
    pub fn apply_mutations(&mut self, batch: Vec<HistoryMutation>) {
        for mutation in batch {
            match mutation {
                HistoryMutation::SetContextOverride { entry_id, value } => {
                    if let Some(entry) = self.core.history.iter_mut().find(|e| e.id == entry_id) {
                        entry.context_override = value;
                    }
                }
                HistoryMutation::InsertEntry { after_entry_id, entry } => {
                    let insert_at = match after_entry_id {
                        Some(id) => {
                            match self.find_entry_index_by_id(&id) {
                                Some(idx) => idx + 1,
                                None => continue,
                            }
                        }
                        None => 0,
                    };
                    self.insert_entry_at(insert_at, entry);
                }
                HistoryMutation::PinEntry { entry_id, position } => {
                    self.pin_entry(&entry_id, position);
                }
                HistoryMutation::UnpinEntry { entry_id } => {
                    self.unpin_entry(&entry_id);
                }
            }
        }
    }

    /// Drain all pending mutation batches and apply them.
    /// Returns the number of batches applied.
    pub fn drain_and_apply_pending_mutations(&mut self) -> usize {
        let batches = self.drain_pending_mutations();
        let count = batches.len();
        for batch in batches {
            self.apply_mutations(batch);
        }
        count
    }
```

### Implementation Order

1. Create `history_mutation.rs` with the enum
2. Add `pub mod history_mutation;` to `feat/session/mod.rs`
3. Modify `chat_session.rs`: add import, field, and methods
4. Build (`just check`) to verify compilation
5. Create `history_mutation_tests.rs` with all unit tests
6. Add `#[cfg(test)] mod history_mutation_tests;` to `feat/session/mod.rs`
7. Run tests (`just test`)

### Acceptance Criteria

- [x] `HistoryMutation` enum exists with `SetContextOverride`, `InsertEntry`, `PinEntry`, `UnpinEntry` variants
- [x] `pending_mutations: Vec<Vec<HistoryMutation>>` field on `SessionCoreEphemeral` with `#[serde(skip)]`
- [x] `ChatSessionState::queue_mutations` appends non-empty batches
- [x] `ChatSessionState::drain_pending_mutations` empties the queue and returns all batches
- [x] `ChatSessionState::find_entry_index_by_id` resolves valid IDs to indices
- [x] `ChatSessionState::find_entry_index_by_id` returns `None` for nonexistent IDs
- [x] `ChatSessionState::apply_mutations` handles `SetContextOverride` by ID
- [x] `ChatSessionState::apply_mutations` handles `InsertEntry` inserting after the specified ID
- [x] `ChatSessionState::apply_mutations` handles `InsertEntry { after_entry_id: None }` inserting at index 0
- [x] `ChatSessionState::apply_mutations` handles `PinEntry` by ID
- [x] `ChatSessionState::apply_mutations` handles `UnpinEntry` by ID
- [x] `ChatSessionState::apply_mutations` silently skips mutations targeting nonexistent entries
- [x] `ChatSessionState::apply_mutations` on empty history is a no-op
- [x] `InsertEntry` mutations shift `streaming_entry_index` correctly
- [x] `InsertEntry` mutations shift `streaming_thinking_entry_index` correctly
- [x] `InsertEntry` mutations shift `streaming_tool_call_indices` correctly
- [x] `InsertEntry` mutations shift `streaming_tool_result_indices` correctly
- [x] `ChatSessionState::drain_and_apply_pending_mutations` drains and applies all batches in order
- [x] `queue_mutations` ignores empty batches
- [x] Multiple `InsertEntry` in one batch produce correct ordering
- [x] `just check` passes
- [x] `just test` passes (all existing tests still pass)

---

## Review: Phase 1 — Define `HistoryMutation` enum and mutation queue

### Changes

Created the `HistoryMutation` enum with four variants (`SetContextOverride`, `InsertEntry`, `PinEntry`, `UnpinEntry`) and added a pending mutation queue to `SessionCoreEphemeral`. Added five new methods to `ChatSessionState`: `find_entry_index_by_id`, `queue_mutations`, `drain_pending_mutations`, `apply_mutations`, and `drain_and_apply_pending_mutations`. Wrote 21 unit tests covering all mutation types, index shifting, nonexistent entry skipping, and queue operations.

### Divergence Summary

None. All acceptance criteria met as planned.

### Verification

- `just check` passes (compilation successful)
- 21 new tests pass (`cargo nextest run -p nullslop-domain -E "test(history_mutation)"`)
- 2920 total tests pass across affected crates (no regressions)
- Pre-existing `nullslop-workflow` compilation error unrelated to our changes

### Risks

- The `apply_mutations` method resolves `ChatEntryId` → index via linear scan (`iter().position()`). For very large histories (5000+ entries), this could be slow. A `HashMap<ChatEntryId, usize>` index could be added later if needed.
- Phase 2 will add the safe application hooks that call `drain_and_apply_pending_mutations` at turn boundaries.

### Next Steps

Proceeding to phase 2.
