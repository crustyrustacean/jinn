//! Tests for the properties popup's save path: the arming flow, the
//! untitled refusal, and the write itself, driven through route dispatch
//! against a real `AppState` and a real `jinn.toml` document.

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code"
)]

use std::sync::Arc;

use jinn_app_state::AppState;
use jinn_attendant_msg::{
    AttendantPropertiesState, OriginalValues, PopupStatus, attendant_properties_scope,
    attendant_properties_slot,
};
use jinn_config::{ConfigLayer, InMemoryConfigStorage};
use jinn_core_types::{ChatEntry, PinPosition, SessionId};
use jinn_preferences_config::schemas::AttendantEntryConfig;
use jinn_session_state::ChatSessionState;
use jinn_slices::KeyRoutes;
use jinn_slices::cell::TypedCell;
use jinn_slices::route::{ActionCtx, DynamicIntent, ScopeSignal};

use crate::properties_overlay::{attach_properties_rows, attach_seed_template_rows};

/// An AppState with one titled attendant, a popup cell seeded from it, and
/// a properties document the save writes to.
struct SaveFixture {
    state: AppState,
    slices: jinn_slices::Slices,
    routes: KeyRoutes,
    cell: TypedCell<AttendantPropertiesState>,
    config: ConfigLayer,
    attendant_id: SessionId,
}

impl SaveFixture {
    /// Builds the fixture, optionally titling the attendant. An untitled
    /// attendant is the default because that is the refusal case; a title
    /// is what makes an attendant savable at all.
    fn new(title: Option<&str>) -> Self {
        let doc = "# user's own comment\n".parse().expect("parses");
        let config = ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(doc)))
            .expect("an empty document always loads");
        let mut state = AppState::default_with_scope_focus();
        let attendant_id = {
            let parent = ChatSessionState::new();
            let mut attendant = ChatSessionState::new_attendant(&parent, true);
            // Only a real title is stamped: an empty one is still an
            // untitled session, which is the case the save refuses.
            if let Some(title) = title {
                attendant.set_title(title.to_owned());
            }
            let id = attendant.session_id().clone();
            state.session.insert(attendant);
            state.session.set_active(id.clone());
            id
        };
        let slices = jinn_slices::Slices::new();
        let cell = slices
            .register(
                attendant_properties_slot(),
                AttendantPropertiesState::default(),
            )
            .expect("unclaimed slot");
        let routes = KeyRoutes::new();
        attach_properties_rows(&routes, &cell);
        crate::properties_overlay::attach_seed_template_rows(&routes, &cell);
        crate::properties_overlay::register_seed_template_input_hook(&routes, &cell);
        attach_seed_template_rows(&routes, &cell);
        Self {
            state,
            slices,
            routes,
            cell,
            config,
            attendant_id,
        }
    }

    /// Opens the popup over the attendant, as the sidebar opener does.
    fn open(&mut self) {
        let session = self
            .state
            .session
            .get(&self.attendant_id)
            .expect("attendant");
        let template = session.seed_template().to_owned();
        let cursor_pos = template.len();
        let popup = AttendantPropertiesState {
            session_id: Some(self.attendant_id.clone()),
            seed_template: jinn_slices::LineInput {
                input: template.clone(),
                cursor_pos,
            },
            pending_behavior: session.attendant_behavior(),
            pending_trigger: session.attendant_trigger(),
            original: Some(OriginalValues {
                trigger: session.attendant_trigger(),
                behavior: session.attendant_behavior(),
                prep_mode: session.attendant_is_prepping(),
                template,
            }),
            ..AttendantPropertiesState::default()
        };
        self.cell.update(|s| *s = popup);
        self.state
            .frontend
            .scope_push(jinn_slices::FocusScope::Dynamic(
                attendant_properties_scope(),
            ));
    }

    /// Runs a properties row and applies any scope signal.
    fn press(&mut self, action: &'static str) -> jinn_slices::RouteResult {
        let ctx = ActionCtx {
            state: &mut self.state,
            slices: &self.slices,
            config: &self.config,
            key_bytes: Vec::new(),
        };
        let intent = DynamicIntent::new(attendant_properties_scope(), action, "test");
        let result = self
            .routes
            .action_for(&intent, ctx)
            .unwrap_or_else(|| panic!("no row bound for action {action:?}"));
        if let Some(ScopeSignal::PopIf(scope)) = result.scope_signal.clone()
            && self.state.frontend.scope() == jinn_slices::FocusScope::Dynamic(scope)
        {
            self.state.frontend.scope_pop();
        }
        result
    }

    /// Seeds the document with an entry already saved under `name`, as a
    /// previous run of this attendant would have left it.
    fn presaved(&mut self, name: &str) {
        let mut session = self
            .state
            .session
            .get(&self.attendant_id)
            .expect("attendant")
            .clone();
        session.set_title(name.to_owned());
        let entry = crate::saved_entry::entry_for_session(name.to_owned(), &session);
        self.config
            .put_list::<AttendantEntryConfig>(&[entry])
            .expect("the fixture document is always writable");
    }

    /// The saved entries, read back from the document.
    fn saved(&self) -> Vec<AttendantEntryConfig> {
        self.config
            .get_list::<AttendantEntryConfig>()
            .expect("list reads")
    }

    /// Pushes a pinned entry into the attendant's history.
    fn add_pin(&mut self, text: &str, position: PinPosition) {
        let entry = ChatEntry {
            pin_position: Some(position),
            ..ChatEntry::user(text)
        };
        self.state
            .session
            .get_mut(&self.attendant_id)
            .expect("attendant")
            .push_entry(entry);
    }

    /// Every key the popup binds, as a composition probe would see them.
    fn bound_keys(&self) -> Vec<&'static str> {
        self.routes.rows().into_iter().map(|row| row.key).collect()
    }
}

#[rstest::rstest]
#[test]
fn an_untitled_attendant_cannot_be_saved() {
    // Given an open popup over an attendant that has never been titled.
    let mut fx = SaveFixture::new(None);
    fx.open();

    // When pressing the save key.
    fx.press("attendant-properties-save");

    // Then nothing is written — a session with no title has no identity to
    // save under.
    assert!(fx.saved().is_empty());
}

#[rstest::rstest]
#[test]
fn an_untitled_refusal_says_so_in_the_attendants_own_log() {
    // Given an open popup over an untitled attendant.
    let mut fx = SaveFixture::new(None);
    fx.open();

    // When pressing the save key.
    let result = fx.press("attendant-properties-save");

    // Then a line is published to the attendant itself, so the user learns
    // why nothing happened rather than concluding the key is broken.
    assert_eq!(result.message_names, vec!["PushChatEntry"]);
}

#[rstest::rstest]
#[test]
fn a_new_attendant_saves_on_one_press() {
    // Given an open popup over a titled attendant no entry shares.
    let mut fx = SaveFixture::new(Some("nightly"));
    fx.open();

    // When pressing the save key once.
    fx.press("attendant-properties-save");

    // Then the entry is written immediately — a first save destroys
    // nothing, so it needs no second press.
    let saved = fx.saved();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "nightly");
}

#[rstest::rstest]
#[test]
fn an_existing_name_arms_on_the_first_press_and_writes_nothing() {
    // Given an open popup over an attendant whose name an entry already
    // holds.
    let mut fx = SaveFixture::new(Some("nightly"));
    fx.open();
    let mut seeded = fx.saved();
    let existing = AttendantEntryConfig {
        name: "nightly".to_owned(),
        seed_template: "the original".to_owned(),
        ..AttendantEntryConfig::default()
    };
    seeded.push(existing);
    fx.config
        .put_list::<AttendantEntryConfig>(&seeded)
        .expect("seed writes");

    // When pressing the save key once.
    fx.press("attendant-properties-save");

    // Then nothing is replaced yet.
    assert_eq!(fx.saved()[0].seed_template, "the original");
    // And the popup reports itself armed.
    assert!(fx.cell.read().save_armed);
}

#[rstest::rstest]
#[test]
fn a_second_press_replaces_the_armed_entry() {
    // Given a popup armed against an existing same-named entry.
    let mut fx = SaveFixture::new(Some("nightly"));
    fx.open();
    let mut seeded = fx.saved();
    seeded.push(AttendantEntryConfig {
        name: "nightly".to_owned(),
        seed_template: "the original".to_owned(),
        ..AttendantEntryConfig::default()
    });
    fx.config
        .put_list::<AttendantEntryConfig>(&seeded)
        .expect("seed writes");
    fx.press("attendant-properties-save");
    assert!(fx.cell.read().save_armed);

    // When pressing the save key again.
    fx.press("attendant-properties-save");

    // Then the entry is replaced rather than duplicated.
    let saved = fx.saved();
    assert_eq!(saved.len(), 1);
    assert_ne!(saved[0].seed_template, "the original");
    // And the popup is unarmed again.
    assert!(!fx.cell.read().save_armed);
}

#[rstest::rstest]
#[test]
fn an_armed_popup_does_not_survive_a_cancel() {
    // Given a popup armed against an existing entry.
    let mut fx = SaveFixture::new(Some("nightly"));
    fx.open();
    let mut seeded = fx.saved();
    seeded.push(AttendantEntryConfig {
        name: "nightly".to_owned(),
        ..AttendantEntryConfig::default()
    });
    fx.config
        .put_list::<AttendantEntryConfig>(&seeded)
        .expect("seed writes");
    fx.press("attendant-properties-save");
    assert!(fx.cell.read().save_armed);

    // When leaving with esc.
    fx.press("attendant-properties-leave");

    // Then the arm is gone: reopening the popup must not offer to destroy
    // an entry the user never confirmed.
    assert!(!fx.cell.read().save_armed);
}

#[rstest::rstest]
#[test]
fn an_armed_popup_does_not_survive_an_apply() {
    // Given a popup armed against an existing entry.
    let mut fx = SaveFixture::new(Some("nightly"));
    fx.open();
    let mut seeded = fx.saved();
    seeded.push(AttendantEntryConfig {
        name: "nightly".to_owned(),
        ..AttendantEntryConfig::default()
    });
    fx.config
        .put_list::<AttendantEntryConfig>(&seeded)
        .expect("seed writes");
    fx.press("attendant-properties-save");

    // When applying the pending fields and closing.
    fx.press("attendant-properties-apply");

    // Then the arm is gone with the popup.
    assert!(!fx.cell.read().save_armed);
}

#[rstest::rstest]
#[test]
fn a_save_captures_the_attendants_pins_in_order() {
    // Given a titled attendant with two pinned entries and one unpinned
    // one between them.
    let mut fx = SaveFixture::new(Some("nightly"));
    fx.add_pin("first instruction", PinPosition::Top);
    let session_id = fx.attendant_id.clone();
    fx.state
        .session
        .get_mut(&session_id)
        .expect("attendant")
        .push_entry(ChatEntry::user("chatter"));
    fx.add_pin("second instruction", PinPosition::Bottom);
    fx.open();

    // When saving.
    fx.press("attendant-properties-save");

    // Then the entry carries both pins, in history order, and not the
    // unpinned entry.
    let saved = fx.saved();
    let texts: Vec<String> = saved[0].pins.iter().map(ChatEntry::text).collect();
    assert_eq!(texts, vec!["first instruction", "second instruction"]);
    assert_eq!(saved[0].pins[0].pin_position, Some(PinPosition::Top));
    assert_eq!(saved[0].pins[1].pin_position, Some(PinPosition::Bottom));
}

#[rstest::rstest]
#[test]
fn a_save_preserves_the_documents_comments() {
    // Given a document carrying a user comment.
    let mut fx = SaveFixture::new(Some("nightly"));
    fx.open();

    // When saving.
    fx.press("attendant-properties-save");

    // Then the comment is still there — the save went through the config
    // layer's list API, which patches rather than re-serializes.
    assert!(
        fx.config.document_text().contains("# user's own comment"),
        "comment lost:\n{}",
        fx.config.document_text()
    );
}

#[rstest::rstest]
#[test]
fn a_save_records_the_sessions_run_configuration() {
    // Given a titled attendant whose trigger is live and whose behavior
    // is reset.
    let mut fx = SaveFixture::new(Some("nightly"));
    {
        let session = fx
            .state
            .session
            .get_mut(&fx.attendant_id)
            .expect("attendant");
        session.set_attendant_behavior(jinn_attendant_msg::AttendantBehavior::Reset);
        session.set_attendant_trigger(jinn_attendant_msg::AttendantTrigger::ParentCompleted);
        session.set_seed_template("review: <prior report>".to_owned());
    }
    fx.open();

    // When saving.
    fx.press("attendant-properties-save");

    // Then the entry carries the same run configuration, so a created
    // attendant is live from the first run.
    let saved = fx.saved();
    assert_eq!(
        saved[0].behavior,
        jinn_attendant_msg::AttendantBehavior::Reset
    );
    assert_eq!(
        saved[0].trigger,
        jinn_attendant_msg::AttendantTrigger::ParentCompleted
    );
    assert_eq!(saved[0].seed_template, "review: <prior report>");
}

#[rstest::rstest]
#[test]
fn the_save_key_binds_on_the_properties_scope_only() {
    // Given the popup's attached rows.
    let fx = SaveFixture::new(Some("nightly"));

    // When collecting every key the popup binds.
    let keys = fx.bound_keys();

    // Then the save key is among them, exactly once — a second binding
    // would make which stroke wins depend on attach order.
    assert_eq!(
        keys.iter().filter(|key| **key == "<c-s>").count(),
        1,
        "the save key must bind once in the popup: {keys:?}"
    );
}

// ── The status line ────────────────────────────────────────────────

#[rstest::rstest]
#[test]
fn an_armed_overwrite_is_announced_on_the_status_line() {
    // Given an open popup over a saved attendant, with the same name
    // already in the document.
    let mut fx = SaveFixture::new(Some("nightly"));
    fx.presaved("nightly");
    fx.open();

    // When saving onto the existing entry.
    fx.press("attendant-properties-save");

    // Then the status line asks before replacing it.
    assert_eq!(
        fx.cell.read().status,
        Some(PopupStatus::OverwriteArmed {
            name: "nightly".to_owned()
        })
    );
}

#[rstest::rstest]
#[test]
fn a_committed_save_is_announced_on_the_status_line() {
    // Given an open popup over a new attendant.
    let mut fx = SaveFixture::new(Some("nightly"));
    fx.open();

    // When saving.
    fx.press("attendant-properties-save");

    // Then the status line says it saved.
    assert_eq!(
        fx.cell.read().status,
        Some(PopupStatus::Saved {
            name: "nightly".to_owned()
        })
    );
}

#[rstest::rstest]
#[test]
fn an_untitled_refusal_is_announced_on_the_status_line() {
    // Given an open popup over an attendant with no title.
    let mut fx = SaveFixture::new(None);
    fx.open();

    // When saving.
    fx.press("attendant-properties-save");

    // Then the status line says why nothing was saved.
    match fx.cell.read().status.clone() {
        Some(PopupStatus::SaveFailed { reason }) => {
            assert!(
                reason.contains("no name"),
                "the reason is legible: {reason}"
            );
        }
        other => panic!("the line reports a refusal: {other:?}"),
    }
}

#[rstest::rstest]
#[test]
fn a_moving_the_cursor_clears_the_status_line() {
    // Given a popup whose last key armed an overwrite.
    let mut fx = SaveFixture::new(Some("nightly"));
    fx.open();
    fx.press("attendant-properties-save");
    assert!(fx.cell.read().status.is_some());

    // When moving to the next field.
    fx.press("attendant-properties-field-next");

    // Then the line is empty — it described the last key, not the popup.
    assert_eq!(fx.cell.read().status, None);
}

#[rstest::rstest]
#[test]
fn typing_in_the_template_editor_clears_the_status_line() {
    // Given a popup whose last key armed an overwrite, with the editor
    // open over the seed template.
    let mut fx = SaveFixture::new(Some("nightly"));
    fx.open();
    fx.press("attendant-properties-save");
    fx.press("attendant-properties-field-next");
    fx.press("attendant-properties-field-next");
    fx.press("attendant-properties-edit-template");
    fx.press("attendant-properties-save");
    assert!(fx.cell.read().status.is_some());

    // When typing in the editor.
    let hook = crate::properties_overlay::attendant_properties_input_hook(&fx.cell);
    hook(&jinn_slices::EditIntent::InsertChar('x'));

    // Then the line is empty.
    assert_eq!(fx.cell.read().status, None);
}

#[rstest::rstest]
#[test]
fn the_status_line_uses_the_tone_that_matches_the_outcome() {
    // Given a popup, a theme, and one state per outcome.
    let theme = jinn_theme::default_theme();
    let armed = AttendantPropertiesState {
        status: Some(PopupStatus::OverwriteArmed {
            name: "nightly".to_owned(),
        }),
        ..AttendantPropertiesState::default()
    };
    let saved = AttendantPropertiesState {
        status: Some(PopupStatus::Saved {
            name: "nightly".to_owned(),
        }),
        ..AttendantPropertiesState::default()
    };
    let refused = AttendantPropertiesState {
        status: Some(PopupStatus::SaveFailed {
            reason: "no".to_owned(),
        }),
        ..AttendantPropertiesState::default()
    };

    // When rendering each line.
    let line =
        |popup: &AttendantPropertiesState| crate::properties_overlay::status_line(popup, &theme);

    // Then each is in its outcome's theme color.
    let color = |popup: &AttendantPropertiesState| {
        line(popup)
            .spans
            .first()
            .and_then(|span| span.style.fg)
            .expect("the line is styled as one span")
    };
    assert_eq!(color(&armed), theme.warning);
    assert_eq!(color(&saved), theme.success);
    assert_eq!(color(&refused), theme.error_text);
}

#[rstest::rstest]
#[test]
fn a_malformed_document_refuses_the_save_rather_than_replacing_the_list() {
    // Given a document whose entry names a trigger variant that does not
    // exist, so the list cannot be read back.
    let malformed = r#"
[[attendant.entry]]
name = "nightly"
trigger = "parent-completed"
"#;
    let doc = format!("# user's own comment\n{malformed}")
        .parse()
        .expect("parses");
    let mut fx = SaveFixture::new(Some("nightly"));
    fx.config = ConfigLayer::load(Arc::new(InMemoryConfigStorage::new(doc))).expect("loads");
    fx.open();

    // When pressing the save key.
    fx.press("attendant-properties-save");

    // Then the popup says the save failed, rather than reporting
    // success for a write that would have replaced the unreadable
    // entries with just this one.
    assert!(
        matches!(fx.cell.read().status, Some(PopupStatus::SaveFailed { .. })),
        "the popup reports the failed save, got {:?}",
        fx.cell.read().status
    );
    // And the document still holds the entry the save must not have
    // clobbered.
    let after = fx
        .config
        .get_list::<jinn_preferences_config::schemas::AttendantEntryConfig>();
    assert!(after.is_err(), "the document is left as the user wrote it");
}

#[rstest::rstest]
#[test]
fn a_keystroke_between_the_two_presses_withdraws_the_confirmation() {
    // Given a popup armed against an existing entry, with the overwrite
    // prompt on its status line.
    let mut fx = SaveFixture::new(Some("nightly"));
    fx.open();
    fx.config
        .put_list::<AttendantEntryConfig>(&[AttendantEntryConfig {
            name: "nightly".to_owned(),
            seed_template: "the original".to_owned(),
            ..AttendantEntryConfig::default()
        }])
        .expect("seed writes");
    fx.press("attendant-properties-save");
    assert!(fx.cell.read().save_armed);

    // When the user presses some other key — moving the field focus, say.
    fx.press("attendant-properties-field-next");

    // Then the arm is withdrawn along with the prompt that offered it, so
    // the save key that follows is an ordinary first press, not a
    // confirmation of something the user was never shown.
    assert!(
        !fx.cell.read().save_armed,
        "the offer to overwrite is withdrawn with the prompt that displayed it"
    );
    assert!(fx.cell.read().status.is_none());

    // And pressing the save key again re-arms rather than overwriting.
    fx.press("attendant-properties-save");
    let saved = fx.saved();
    assert_eq!(
        saved[0].seed_template, "the original",
        "nothing was replaced"
    );
    assert!(fx.cell.read().save_armed, "the press re-arms the prompt");
}
