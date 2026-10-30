//! Turn admission for turn dispatch.
//!
//! Every dispatch path mints or joins a turn through the turn ledger before
//! doing anything else. A `None` grant means the dispatch has nothing to
//! join — a cancel landed while the turn sat queued, or no turn is live —
//! and the caller publishes nothing downstream of the mint.

use jinn_core_types::SessionId;
use jinn_kernel::common::services::Services;
use jinn_kernel::common::turn_ledger::MintKind;
use jinn_kernel::common::turn_ledger::TurnGrant;

/// Mints or joins a turn for a dispatch path.
///
/// `FreshTurn` always mints (except during a cancel drain, which it must not
/// clobber); `ResumeTurn` and `ToolContinuation` join the live turn under a
/// fresh stream id. A `None` grant publishes nothing.
#[must_use]
pub fn mint_for_dispatch(
    services: &Services,
    session_id: &SessionId,
    kind: MintKind,
) -> Option<TurnGrant> {
    services.turn_ledger.mint(session_id, kind)
}
