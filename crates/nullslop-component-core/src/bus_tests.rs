use nullslop_protocol as npr;
use npr::chat_input::{DeleteGrapheme, InsertChar};
use npr::provider::SendMessage;
use npr::system::{KeyDown, ModeChanged, Quit, SetMode};

use super::*;
use crate::fake::{FakeCommandHandler, FakeEventHandler};

/// Simple state type for testing bus dispatch.
#[derive(Debug, Default)]
struct TestState;

// --- Command dispatch tests ---

#[test]
fn command_dispatch_reaches_handler() {
    // Given a bus with a handler for InsertChar.
    let (handler, calls) = FakeCommandHandler::<InsertChar, TestState, ()>::continuing();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<InsertChar, _>(handler);

    // When submitting and processing the command.
    bus.submit_command(Command::InsertChar {
        payload: InsertChar { ch: 'x' },
    });
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // Then the handler was called with the correct payload.
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(calls.borrow()[0].ch, 'x');
}

#[test]
fn multiple_command_handlers_all_run() {
    // Given a bus with two handlers for the same command type.
    let (h1, calls1) = FakeCommandHandler::<Quit, TestState, ()>::continuing();
    let (h2, calls2) = FakeCommandHandler::<Quit, TestState, ()>::continuing();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<Quit, _>(h1);
    bus.register_command_handler::<Quit, _>(h2);

    // When processing a command.
    bus.submit_command(Command::Quit);
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // Then both handlers were called.
    assert_eq!(calls1.borrow().len(), 1);
    assert_eq!(calls2.borrow().len(), 1);
}

#[test]
fn stop_halts_propagation() {
    // Given a bus where the first handler returns Stop.
    let (stopper, stopper_calls) = FakeCommandHandler::<Quit, TestState, ()>::stopping();
    let (continuer, continuer_calls) = FakeCommandHandler::<Quit, TestState, ()>::continuing();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<Quit, _>(stopper);
    bus.register_command_handler::<Quit, _>(continuer);

    // When processing a command.
    bus.submit_command(Command::Quit);
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // Then only the first handler was called.
    assert_eq!(stopper_calls.borrow().len(), 1);
    assert!(continuer_calls.borrow().is_empty());
}

#[test]
fn continue_allows_propagation() {
    // Given a bus where the first handler returns Continue.
    let (c1, calls1) = FakeCommandHandler::<Quit, TestState, ()>::continuing();
    let (c2, calls2) = FakeCommandHandler::<Quit, TestState, ()>::continuing();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<Quit, _>(c1);
    bus.register_command_handler::<Quit, _>(c2);

    // When processing a command.
    bus.submit_command(Command::Quit);
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // Then both handlers were called.
    assert_eq!(calls1.borrow().len(), 1);
    assert_eq!(calls2.borrow().len(), 1);
}

#[test]
fn unregistered_command_is_ignored() {
    // Given a bus with no handlers.
    let mut bus: Bus<TestState, ()> = Bus::new();

    // When submitting a command.
    bus.submit_command(Command::Quit);
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // Then no panic occurs and the queue is empty.
    assert!(!bus.has_pending());
}

#[test]
fn unit_command_dispatches_correctly() {
    // Given a bus with a handler for DeleteGrapheme (unit struct).
    let (handler, calls) = FakeCommandHandler::<DeleteGrapheme, TestState, ()>::continuing();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<DeleteGrapheme, _>(handler);

    // When processing a unit command.
    bus.submit_command(Command::DeleteGrapheme);
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // Then the handler was called.
    assert_eq!(calls.borrow().len(), 1);
}

// --- Event dispatch tests ---

#[test]
fn event_dispatch_reaches_handler() {
    // Given a bus with a handler for KeyDown.
    let (handler, calls) = FakeEventHandler::<KeyDown, TestState, ()>::new();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_event_handler::<KeyDown, _>(handler);

    // When processing an event.
    let key = npr::KeyEvent {
        key: npr::Key::Char('a'),
        modifiers: npr::Modifiers::none(),
    };
    bus.submit_event(Event::KeyDown {
        payload: KeyDown { key },
    });
    let mut state = TestState;
    let services = ();
    bus.process_events(&mut state, &services);

    // Then the handler was called.
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn all_event_handlers_run() {
    // Given a bus with two event handlers for ModeChanged.
    let (h1, calls1) = FakeEventHandler::<ModeChanged, TestState, ()>::new();
    let (h2, calls2) = FakeEventHandler::<ModeChanged, TestState, ()>::new();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_event_handler::<ModeChanged, _>(h1);
    bus.register_event_handler::<ModeChanged, _>(h2);

    // When processing an event.
    bus.submit_event(Event::ModeChanged {
        payload: ModeChanged {
            from: npr::Mode::Normal,
            to: npr::Mode::Input,
        },
    });
    let mut state = TestState;
    let services = ();
    bus.process_events(&mut state, &services);

    // Then both handlers were called.
    assert_eq!(calls1.borrow().len(), 1);
    assert_eq!(calls2.borrow().len(), 1);
}

// --- Out / cascading tests ---

/// Handler that submits an `AppQuit` command when it sees `InsertChar`.
struct CascadeHandler;

impl CommandHandler<InsertChar, TestState, ()> for CascadeHandler {
    fn handle(
        &self,
        _cmd: &InsertChar,
        ctx: &mut HandlerContext<'_, TestState, ()>,
    ) -> CommandAction {
        ctx.out.submit_command(Command::Quit);
        CommandAction::Continue
    }
}

#[test]
fn cascading_commands_are_processed() {
    // Given a bus where InsertChar handler submits AppQuit.
    let (quit_handler, quit_calls) = FakeCommandHandler::<Quit, TestState, ()>::continuing();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<InsertChar, _>(CascadeHandler);
    bus.register_command_handler::<Quit, _>(quit_handler);

    // When processing the initial command.
    bus.submit_command(Command::InsertChar {
        payload: InsertChar { ch: 'x' },
    });
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // Then the cascaded Quit was also processed.
    assert_eq!(quit_calls.borrow().len(), 1);
}

/// Handler that resubmits itself, creating a potential infinite loop.
struct LoopHandler;

impl CommandHandler<InsertChar, TestState, ()> for LoopHandler {
    fn handle(
        &self,
        _cmd: &InsertChar,
        ctx: &mut HandlerContext<'_, TestState, ()>,
    ) -> CommandAction {
        ctx.out.submit_command(Command::InsertChar {
            payload: InsertChar { ch: 'x' },
        });
        CommandAction::Continue
    }
}

#[test]
fn max_iterations_prevents_infinite_loop() {
    // Given a bus where the handler resubmits itself, with a low max_iterations.
    let mut bus: Bus<TestState, ()> = Bus::new().with_max_iterations(3);
    bus.register_command_handler::<InsertChar, _>(LoopHandler);

    // When processing commands.
    bus.submit_command(Command::InsertChar {
        payload: InsertChar { ch: 'x' },
    });
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // Then it terminates without hanging.
}

// --- has_pending tests ---

#[test]
fn has_pending_is_false_before_command_submit() {
    // Given an empty bus.
    let mut bus: Bus<TestState, ()> = Bus::new();

    // Then the bus has no pending messages.
    assert!(!bus.has_pending());
}

#[test]
fn has_pending_is_true_after_command_submit() {
    // Given an empty bus.
    let mut bus: Bus<TestState, ()> = Bus::new();

    // When submitting a command.
    bus.submit_command(Command::Quit);

    // Then the bus has pending messages.
    assert!(bus.has_pending());
}

#[test]
fn has_pending_is_false_after_command_process() {
    // Given a bus with a submitted command.
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.submit_command(Command::Quit);

    // When processing commands.
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // Then the bus has no pending messages again.
    assert!(!bus.has_pending());
}

#[test]
fn has_pending_is_false_before_event_submit() {
    // Given an empty bus.
    let mut bus: Bus<TestState, ()> = Bus::new();

    // Then the bus has no pending messages.
    assert!(!bus.has_pending());
}

#[test]
fn has_pending_is_true_after_event_submit() {
    // Given an empty bus.
    let mut bus: Bus<TestState, ()> = Bus::new();

    // When submitting an event.
    bus.submit_event(Event::ModeChanged {
        payload: ModeChanged {
            from: npr::Mode::Normal,
            to: npr::Mode::Input,
        },
    });

    // Then the bus has pending messages.
    assert!(bus.has_pending());
}

#[test]
fn has_pending_is_false_after_event_process() {
    // Given a bus with a submitted event.
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.submit_event(Event::ModeChanged {
        payload: ModeChanged {
            from: npr::Mode::Normal,
            to: npr::Mode::Input,
        },
    });

    // When processing events.
    let mut state = TestState;
    let services = ();
    bus.process_events(&mut state, &services);

    // Then the bus has no pending messages again.
    assert!(!bus.has_pending());
}

// --- Mixed dispatch: struct variant with payload ---

#[test]
fn struct_command_with_payload_dispatches() {
    // Given a bus with handlers for multiple struct commands.
    let (set_mode_handler, set_mode_calls) =
        FakeCommandHandler::<SetMode, TestState, ()>::continuing();
    let (send_handler, send_calls) =
        FakeCommandHandler::<SendMessage, TestState, ()>::continuing();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<SetMode, _>(set_mode_handler);
    bus.register_command_handler::<SendMessage, _>(send_handler);

    // When submitting multiple commands.
    bus.submit_command(Command::SetMode {
        payload: SetMode {
            mode: npr::Mode::Input,
        },
    });
    bus.submit_command(Command::SendMessage {
        payload: SendMessage {
            session_id: npr::SessionId::new(),
            text: "hello".into(),
        },
    });
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // Then both handlers were called with correct payloads.
    assert_eq!(set_mode_calls.borrow().len(), 1);
    assert_eq!(send_calls.borrow().len(), 1);
    assert_eq!(send_calls.borrow()[0].text, "hello");
}

/// Handler that submits an event when processing a command.
struct CommandToEventHandler;

impl CommandHandler<Quit, TestState, ()> for CommandToEventHandler {
    fn handle(
        &self,
        _cmd: &Quit,
        ctx: &mut HandlerContext<'_, TestState, ()>,
    ) -> CommandAction {
        ctx.out.submit_event(Event::ModeChanged {
            payload: ModeChanged {
                from: npr::Mode::Normal,
                to: npr::Mode::Input,
            },
        });
        CommandAction::Continue
    }
}

#[test]
fn command_handler_submitted_event_queues_before_processing() {
    // Given a bus where Quit handler submits ModeChanged.
    let (_event_handler, _event_calls) = FakeEventHandler::<ModeChanged, TestState, ()>::new();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<Quit, _>(CommandToEventHandler);
    bus.register_event_handler::<ModeChanged, _>(_event_handler);

    // When processing a command that submits an event.
    bus.submit_command(Command::Quit);
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // Then the event is in the event queue (not yet processed).
    assert!(bus.has_pending());
}

#[test]
fn queued_events_reach_handler_on_process() {
    // Given a bus where Quit handler submits ModeChanged.
    let (event_handler, event_calls) = FakeEventHandler::<ModeChanged, TestState, ()>::new();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<Quit, _>(CommandToEventHandler);
    bus.register_event_handler::<ModeChanged, _>(event_handler);

    bus.submit_command(Command::Quit);
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // When processing events.
    bus.process_events(&mut state, &services);

    // Then the event handler was called.
    assert_eq!(event_calls.borrow().len(), 1);
}

// --- Drain processed tests ---

#[test]
fn drain_returns_command_and_event() {
    // Given a bus with a command and event handler.
    let (cmd_handler, _cmd_calls) = FakeCommandHandler::<Quit, TestState, ()>::continuing();
    let (evt_handler, _evt_calls) = FakeEventHandler::<ModeChanged, TestState, ()>::new();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<Quit, _>(cmd_handler);
    bus.register_event_handler::<ModeChanged, _>(evt_handler);

    // When processing a command and event.
    bus.submit_command(Command::Quit);
    bus.submit_event(Event::ModeChanged {
        payload: ModeChanged {
            from: npr::Mode::Normal,
            to: npr::Mode::Input,
        },
    });
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);
    bus.process_events(&mut state, &services);

    // Then drain returns both.
    let events = bus.drain_processed_events();
    let commands = bus.drain_processed_commands();
    assert_eq!(events.len(), 1);
    assert!(matches!(events[0].event, Event::ModeChanged { .. }));
    assert_eq!(commands.len(), 1);
    assert!(matches!(commands[0].command, Command::Quit));
}

#[test]
fn drain_returns_items_without_source() {
    // Given a bus with a command and event handler.
    let (cmd_handler, _cmd_calls) = FakeCommandHandler::<Quit, TestState, ()>::continuing();
    let (evt_handler, _evt_calls) = FakeEventHandler::<ModeChanged, TestState, ()>::new();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<Quit, _>(cmd_handler);
    bus.register_event_handler::<ModeChanged, _>(evt_handler);

    bus.submit_command(Command::Quit);
    bus.submit_event(Event::ModeChanged {
        payload: ModeChanged {
            from: npr::Mode::Normal,
            to: npr::Mode::Input,
        },
    });
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);
    bus.process_events(&mut state, &services);

    // Then drain returns items with no source.
    let events = bus.drain_processed_events();
    let commands = bus.drain_processed_commands();
    assert!(events[0].source.is_none());
    assert!(commands[0].source.is_none());
}

#[test]
fn first_drain_returns_items() {
    // Given a bus with a command and event handler.
    let (cmd_handler, _cmd_calls) = FakeCommandHandler::<Quit, TestState, ()>::continuing();
    let (evt_handler, _evt_calls) = FakeEventHandler::<ModeChanged, TestState, ()>::new();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<Quit, _>(cmd_handler);
    bus.register_event_handler::<ModeChanged, _>(evt_handler);
    bus.submit_command(Command::Quit);
    bus.submit_event(Event::ModeChanged {
        payload: ModeChanged {
            from: npr::Mode::Normal,
            to: npr::Mode::Input,
        },
    });
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);
    bus.process_events(&mut state, &services);

    // When draining.
    let first_events = bus.drain_processed_events();
    let first_commands = bus.drain_processed_commands();

    // Then first has items.
    assert_eq!(first_events.len(), 1);
    assert_eq!(first_commands.len(), 1);
}

#[test]
fn second_drain_returns_empty() {
    // Given a bus with a command and event handler.
    let (cmd_handler, _cmd_calls) = FakeCommandHandler::<Quit, TestState, ()>::continuing();
    let (evt_handler, _evt_calls) = FakeEventHandler::<ModeChanged, TestState, ()>::new();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<Quit, _>(cmd_handler);
    bus.register_event_handler::<ModeChanged, _>(evt_handler);
    bus.submit_command(Command::Quit);
    bus.submit_event(Event::ModeChanged {
        payload: ModeChanged {
            from: npr::Mode::Normal,
            to: npr::Mode::Input,
        },
    });
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);
    bus.process_events(&mut state, &services);

    // When draining twice.
    bus.drain_processed_events();
    bus.drain_processed_commands();
    let second_events = bus.drain_processed_events();
    let second_commands = bus.drain_processed_commands();

    // Then second is empty.
    assert!(second_events.is_empty());
    assert!(second_commands.is_empty());
}

// --- Source tagging tests ---

#[test]
fn submit_command_from_preserves_source() {
    // Given a bus with a command handler.
    let (handler, _calls) = FakeCommandHandler::<Quit, TestState, ()>::continuing();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<Quit, _>(handler);

    // When submitting a command with a source.
    bus.submit_command_from(Command::Quit, Some(ActorName::new("ext-test")));
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // Then the source is preserved through drain.
    let processed = bus.drain_processed_commands();
    assert_eq!(processed.len(), 1);
    assert_eq!(processed[0].source.as_deref(), Some("ext-test"));
}

#[test]
fn submit_event_from_preserves_source() {
    // Given a bus with an event handler.
    let (handler, _calls) = FakeEventHandler::<ModeChanged, TestState, ()>::new();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_event_handler::<ModeChanged, _>(handler);

    // When submitting an event with a source.
    bus.submit_event_from(
        Event::ModeChanged {
            payload: ModeChanged {
                from: npr::Mode::Normal,
                to: npr::Mode::Input,
            },
        },
        Some(ActorName::new("ext-test")),
    );
    let mut state = TestState;
    let services = ();
    bus.process_events(&mut state, &services);

    // Then the source is preserved through drain.
    let processed = bus.drain_processed_events();
    assert_eq!(processed.len(), 1);
    assert_eq!(processed[0].source.as_deref(), Some("ext-test"));
}

#[test]
fn submit_command_without_source_has_none() {
    // Given a bus with a command handler.
    let (handler, _calls) = FakeCommandHandler::<Quit, TestState, ()>::continuing();
    let mut bus: Bus<TestState, ()> = Bus::new();
    bus.register_command_handler::<Quit, _>(handler);

    // When submitting a command without source.
    bus.submit_command(Command::Quit);
    let mut state = TestState;
    let services = ();
    bus.process_commands(&mut state, &services);

    // Then the source is None.
    let processed = bus.drain_processed_commands();
    assert!(processed[0].source.is_none());
}
