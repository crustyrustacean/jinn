//! Slice vocabulary — typed cells, views, and the slots that name them.
//!
//! Shared application state is guarded by a single `RwLock` while
//! independently owned render data lives in typed cells. [`Slices`]
//! gives each extracted slice its own named cell and typed handles for
//! accessing it, making its data flow explicit at call sites.
//!
//! Keys are dynamic strings ([`SlotKey`]), not an enum of known features,
//! so a feature can register a cell under its own namespace without
//! touching a central registry.
//!
//! Read access is not scarce; write access is.
//!
//! This crate is the extraction seam for slice *features*: it
//! depends only on `jinn-theme` and `ratatui` (for the view layer) —
//! never on `jinn-domain`.

#![cfg_attr(
    test,
    allow(
        clippy::expect_used,
        clippy::panic,
        reason = "test assertions on infallible registration"
    )
)]

pub mod assembled_prompt;
pub mod bus;
pub mod cell;
pub mod cwd_root;
pub mod fabric;
pub mod focus;
pub mod host;
pub mod key;
pub mod line_input;
pub mod mode;
pub mod overlay;
pub mod persona;
pub mod picker_kind;
pub mod render_facts;
pub mod route;
pub mod route_publish;
pub mod scope_focus_state;
pub mod service_status;
pub mod slice_scope;
pub mod slices;
pub mod spinner;
pub mod tui_signals;
pub mod view;

pub use assembled_prompt::AssembledPrompt;
pub use assembled_prompt::SystemPrompt;
pub use cell::TypedCell;
pub use cwd_root::CwdRoot;
pub use fabric::ActorStarted;
pub use fabric::ActorStarting;
pub use focus::{FocusScope, ScopeStack};
pub use host::SliceHost;
pub use key::{Key, KeyEvent, Modifiers};
pub use line_input::LineInput;
pub use mode::Mode;
pub use overlay::OverlayViewFn;
pub use overlay::OverlayViews;
pub use persona::Persona;
pub use picker_kind::PickerKind;
pub use render_facts::AppFact;
pub use render_facts::RenderFacts;
pub use scope_focus_state::ScopeFocusState;
pub use scope_focus_state::scope_focus_slot;
pub use tui_signals::TuiSignals;

/// The slice host specialized to jinn's render facts — the spelling
/// slices use in their `activate` signatures instead of naming the
/// generic parameter everywhere.
pub type AppSliceHost<'a> = SliceHost<'a, RenderFacts>;
pub use jinn_config::ConfigLayer;
pub use jinn_config::empty_config_layer;
pub use route::ActionCtx;
pub use route::ActionFn;
pub use route::BindSite;
pub use route::BusMessage;
pub use route::DynamicIntent;
pub use route::EditIntent;
pub use route::InputHook;
pub use route::KeyHook;
pub use route::KeyRoutes;
pub use route::PublishClosure;
pub use route::PublishableMessage;
pub use route::RouteId;
pub use route::RouteOutcome;
pub use route::RouteResult;
pub use route::RouteRow;
pub use route::ScopeEnterHook;
pub use route::ScopeSignal;
pub use route::SliceActionState;
pub use route_publish::PublishSink;
pub use service_status::ServiceStatusUpdate;
pub use slice_scope::SliceScopeId;
pub use slices::Slices;
pub use slices::SlotKey;
pub use slices::SlotTaken;
pub use spinner::SPINNER_INTERVAL;
pub use spinner::spinner_glyph;
pub use spinner::spinner_index;
pub use view::SliceView;
pub use view::ViewCx;
