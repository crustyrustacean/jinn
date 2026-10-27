//! Turn-path persistence — the session actor's own save and interaction mark.
//!
//! Load, fork, and archive now live in the `jinn-session-store` slice. The
//! turn/context actor still saves directly on its own path: enqueue, streaming,
//! tool calls, pin changes, and task-list updates all mutate the session and
//! must reach disk without routing a command through another actor.

use super::super::SessionPersistenceActor;
use jinn_kernel::common::actor_deps::BusPublish;
use jinn_session_msg::{MarkSessionInteracted, UserInteracted};

impl SessionPersistenceActor {
    /// Saves a coherent snapshot of a session to disk, stamped with the
    /// work-time cell's current record.
    ///
    /// A delegation to the kernel's one stamped-save flow
    /// ([`jinn_kernel::common::session_persistence`]), shared with the
    /// session-store actor so no durable write can store an un-stamped
    /// snapshot. The write lock is held only for the cheap `touch()`
    /// mutation and the capture runs before store I/O, as the flow's own
    /// contract; errors are logged as warnings there — persistence failure
    /// must not break the user experience.
    pub(in crate::session_actor) async fn save_active_session(
        &self,
        session_id: &jinn_core_types::SessionId,
    ) {
        jinn_kernel::common::session_persistence::save_session_stamped(
            &self.services,
            &self.state,
            session_id,
            None,
        )
        .await;
    }

    /// Marks a session as having been interacted with by the user.
    ///
    /// Sets `has_interacted = true` on the session and emits a `UserInteracted` event.
    pub(in crate::session_actor) async fn handle_mark_session_interacted(
        &mut self,
        payload: &MarkSessionInteracted,
    ) {
        self.state.with_session(|view| {
            if let Some(session) = view.session.map().get_mut(&payload.session_id) {
                session.mark_interacted();
            }
        });

        self.publish(UserInteracted {
            session_id: payload.session_id.clone(),
        })
        .await;

        self.save_active_session(&payload.session_id).await;
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::expect_used,
        clippy::panic,
        clippy::unreachable,
        clippy::indexing_slicing,
        reason = "test code"
    )]

    use super::super::super::helpers::test_actor_with_store_recording;
    use jinn_slices::cell::TypedCell;

    /// The fixture's actor plus the work-time cell, registered on the same
    /// `Slices` the actor's Services holds — the catalog does not run in
    /// this harness, so the test registers it itself and keeps the handle
    /// for seeding. The save reads the cell through `services.slices`, so a
    /// cell from any other registry would make the assertions vacuous.
    async fn actor_with_work_time_cell() -> (
        super::super::super::SessionPersistenceActor,
        std::sync::Arc<super::super::super::helpers::PopulatedFakeStore>,
        jinn_kernel::common::services::BusAudit,
        TypedCell<jinn_work_time_msg::WorkingTimeState>,
    ) {
        let (actor, store, audit) = test_actor_with_store_recording(vec![]).await;
        let cell = actor
            .services
            .slices
            .register(
                jinn_work_time_msg::work_time_slot(),
                jinn_work_time_msg::WorkingTimeState::default(),
            )
            .expect("the turn-actor harness registers no work-time slot");
        (actor, store, audit, cell)
    }

    /// The 90s of recorded work the seeded cell holds, as an interval.
    fn recorded_interval() -> jinn_core_types::WorkingInterval {
        jinn_core_types::WorkingInterval::closed(recorded_interval_start(), recorded_interval_end())
    }

    /// When the recorded work began.
    fn recorded_interval_start() -> jiff::Timestamp {
        jiff::Timestamp::from_second(1_000_000_000).expect("valid timestamp")
    }

    /// When the recorded work ended.
    fn recorded_interval_end() -> jiff::Timestamp {
        jiff::Timestamp::from_second(1_000_000_090).expect("valid timestamp")
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_active_session_skips_non_persistable_session() {
        // Given a recording actor whose active session has never been interacted with.
        let (actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();

        // When saving the active session.
        actor.save_active_session(&session_id).await;

        // Then nothing reaches the store.
        assert!(
            store.last_saved_session(&session_id).is_none(),
            "non-interacted session should not be persisted"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_active_session_persists_interacted_session() {
        // Given a recording actor whose active session has been interacted with.
        let (actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();
        {
            let mut state = actor.state.write();
            state.active_session_mut().mark_interacted();
        }

        // When saving the active session.
        actor.save_active_session(&session_id).await;

        // Then the snapshot reaches the store.
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "interacted session should be persisted"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_active_session_persists_post_mutation_history() {
        // Given an interacted session.
        let (actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();
        {
            let mut state = actor.state.write();
            let session = state.active_session_mut();
            session.mark_interacted();
        }

        // When a user entry is added and the turn path saves.
        {
            let mut state = actor.state.write();
            state
                .active_session_mut()
                .push_entry(jinn_core_types::ChatEntry::user("new turn"));
        }
        actor.save_active_session(&session_id).await;

        // Then the store receives that post-mutation history.
        let snapshot = store
            .last_saved_session(&session_id)
            .expect("session snapshot");
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(
            snapshot.entries[0].prompt_text(),
            Some("new turn"),
            "saved snapshot must contain the latest turn"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_active_session_releases_write_lock_before_clone() {
        use parking_lot::Mutex as ParkingMutex;
        use std::sync::Arc;

        // Given an interacted session with a large history, and the store's
        // save probe armed against the actor's state.
        let (actor, store, session_id, can_write) = {
            let (actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
            let session_id = actor.state.read().session.active_session_id().clone();
            {
                let mut state = actor.state.write();
                let session = state.active_session_mut();
                session.mark_interacted();
                // A large history makes the clone window wide enough to probe.
                for i in 0..5000 {
                    session.push_entry(jinn_core_types::ChatEntry::user(format!("msg {i}")));
                }
            }
            let can_write = Arc::new(ParkingMutex::new(true));
            store.arm_write_probe(actor.state.clone(), can_write.clone());
            (actor, store, session_id, can_write)
        };

        // When saving — eight back-to-back saves, each one a full
        // touch → capture-clone → store.save round trip.
        //
        // The lock-release property is asserted per save, not sampled: the
        // probe lives inside the store's `save`, which the flow reaches only
        // after the capture clone has resolved. A `try_write` there fails
        // exactly when the actor's write lock is still held across the
        // clone — the regression this test exists for. The previous version
        // sampled `try_read` from a hot spin loop and asserted a blocked
        // *fraction*, which is a wall-clock claim: under full-suite load the
        // probe's thread and the saves' spawn_blocking captures share a
        // pool and cores, descheduling the probe across writer windows and
        // skewing the sample (observed 12.6% against a 10% floor with the
        // lock genuinely released). One bit per save has no such mode: it
        // is `false` only if the lock really was held.
        for _ in 0..8 {
            actor.save_active_session(&session_id).await;
        }

        // Then the session was saved (touch() ran under the write lock).
        assert!(
            store.last_saved_session(&session_id).is_some(),
            "interacted session should be persisted"
        );
        // And every save found the write lock free: the clone never held it.
        assert!(
            *can_write.lock(),
            "the write lock must be released across the capture clone; \
             some save ran with the lock still held"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn mark_session_interacted_marks_session_interacted() {
        // Given a session that has not been interacted with.
        let (mut actor, _store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();

        // When MarkSessionInteracted is handled.
        actor
            .handle_mark_session_interacted(&jinn_session_msg::MarkSessionInteracted {
                session_id: session_id.clone(),
            })
            .await;

        // Then the session is marked as interacted.
        let state = actor.state.read();
        let session = state.session.get(&session_id).expect("session exists");
        assert!(session.has_interacted());
        assert!(session.is_persistable());
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn mark_session_interacted_publishes_user_interacted() {
        // Given a session that has not been interacted with.
        let (mut actor, _store, audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();

        // When MarkSessionInteracted is handled.
        actor
            .handle_mark_session_interacted(&jinn_session_msg::MarkSessionInteracted {
                session_id: session_id.clone(),
            })
            .await;

        // Then the UserInteracted event is emitted.
        assert!(
            audit.contains_name("UserInteracted"),
            "UserInteracted event should be emitted"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn mark_session_interacted_persists_session_with_its_recorded_intervals() {
        // Given an actor whose work-time cell records 90s of work for the
        // active session — written through the cell exactly as the monitor
        // leaves it.
        let (mut actor, store, _audit, work_time) = actor_with_work_time_cell().await;
        let session_id = actor.state.read().session.active_session_id().clone();
        work_time.update(|working| {
            working.begin_working(session_id.clone(), recorded_interval_start());
            working.end_working(&session_id, recorded_interval_end());
        });

        // When a turn-path trigger persists the session.
        actor
            .handle_mark_session_interacted(&jinn_session_msg::MarkSessionInteracted {
                session_id: session_id.clone(),
            })
            .await;

        // Then the stored blob carries the cell's record — not the empty
        // list a bare capture holds, which would wipe the session's wall
        // time on every turn.
        let snapshot = store
            .last_saved_session(&session_id)
            .expect("session snapshot");
        assert_eq!(
            snapshot.metadata.working_intervals,
            vec![recorded_interval()],
            "a turn-path save must stamp the session's current wall time"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn mark_session_interacted_stores_no_intervals_key_for_a_session_that_never_worked() {
        // Given an actor whose work-time cell is registered but holds no
        // record for the active session.
        let (mut actor, store, _audit, _work_time) = actor_with_work_time_cell().await;
        let session_id = actor.state.read().session.active_session_id().clone();

        // When a turn-path trigger persists the session.
        actor
            .handle_mark_session_interacted(&jinn_session_msg::MarkSessionInteracted {
                session_id: session_id.clone(),
            })
            .await;

        // Then the stored metadata serializes no intervals key at all —
        // zero is recorded as zero, and the blob round-trips as one written
        // before working time existed.
        let snapshot = store
            .last_saved_session(&session_id)
            .expect("session snapshot");
        let blob = serde_json::to_value(&snapshot.metadata).expect("metadata serializes");
        assert!(
            blob.get("working_intervals").is_none(),
            "a session that never worked must store no intervals key, got {blob:?}"
        );
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn save_active_session_degrades_to_no_intervals_when_the_work_time_cell_is_absent() {
        // Given an interacted session whose harness registered no work-time
        // cell at all.
        let (actor, store, _audit) = test_actor_with_store_recording(vec![]).await;
        let session_id = actor.state.read().session.active_session_id().clone();
        {
            let mut state = actor.state.write();
            state.active_session_mut().mark_interacted();
        }

        // When saving the active session.
        actor.save_active_session(&session_id).await;

        // Then the save still succeeds, storing no intervals rather than
        // failing — an absent cell is a working system, not a broken one.
        let snapshot = store
            .last_saved_session(&session_id)
            .expect("session snapshot");
        assert!(
            snapshot.metadata.working_intervals.is_empty(),
            "an absent cell must save no intervals, got {:?}",
            snapshot.metadata.working_intervals
        );
    }
}
