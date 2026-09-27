//! The slice-facing activation surface — one `SliceHost` per
//! `activate()` call.
//!
//! A slice's entire kernel integration is a sequence of registration
//! verbs on this host: mint a cell, spawn a service actor on trouper,
//! attach route rows, register an input hook, register views, tabs,
//! overlays, and read its config section. In-tree Rust slices call the
//! verbs imperatively from `activate(&mut SliceHost, …)`; a future WASM
//! guest host will perform the same verbs from validated manifest
//! messages. Naming the verbs once means the guest activation technique
//! arrives as a consumer of this surface, not a parallel system.
//!
//! The host borrows the kernel's registries for the duration of
//! activation and adds one of its own: the input-hook registry
//! ([`HookRegistry`]). Hooks stage during activation and install into the
//! kernel's [`KeyRoutes`] once every slice has activated.
//!
//! A slice's `jinn.toml` section is not staged here. It reads the
//! configuration layer directly, at the point of use, so a reload is
//! visible without re-wiring.
//!
//! The host is constructed per activation and never stored on
//! `Services`.

use std::sync::Arc;

use trouper::actor::ActorPath;
use trouper::actor::ServiceActor;
use trouper::system::ActorSystem;

use crate::overlay::OverlayViewFn;
use crate::overlay::OverlayViews;
use crate::route::EditIntent;
use crate::route::KeyRoutes;
use crate::route::RouteResult;
use crate::route::RouteRow;
use crate::slice_scope::SliceScopeId;
use crate::slices::Slices;
use crate::slices::SlotKey;
use crate::slices::SlotTaken;
use crate::view::Viewport;

pub mod host_input;
pub mod host_view;

pub use host_view::HostView;

/// The kernel registries a [`SliceHost`] borrows for one activation.
pub struct SliceHost<'a, C: 'static> {
    slices: &'a Slices,
    viewport: &'a mut Viewport,
    overlay_views: &'a OverlayViews<C>,
    key_routes: &'a KeyRoutes,
    system: &'a ActorSystem,
    hooks: host_input::HookRegistry,
}

impl<'a, C: 'static> SliceHost<'a, C> {
    /// Assembles a host over the kernel's registries.
    #[must_use]
    pub fn new(
        slices: &'a Slices,
        viewport: &'a mut Viewport,
        overlay_views: &'a OverlayViews<C>,
        key_routes: &'a KeyRoutes,
        system: &'a ActorSystem,
    ) -> Self {
        Self {
            slices,
            viewport,
            overlay_views,
            key_routes,
            system,
            hooks: host_input::HookRegistry::default(),
        }
    }

    /// The trouper actor system, for slice actors that spawn with
    /// custom builders (cell-injecting `start_with` overrides).
    #[must_use]
    pub fn system(&self) -> &'a ActorSystem {
        self.system
    }

    /// The kernel's key-route table, for slices that attach rows with
    /// captured cell handles (the row actions close over them).
    #[must_use]
    pub fn key_routes(&self) -> &'a KeyRoutes {
        self.key_routes
    }

    /// The kernel's view registry, for slices that register a typed tab
    /// view alongside their cell. The view/slot pairing is verified by
    /// [`Viewport::register`] against the slices registry, so a missing
    /// cell or a wrong payload type fails here, at launch.
    ///
    /// [`Viewport::register`]: crate::view::Viewport::register
    pub fn viewport(&mut self) -> &mut Viewport {
        self.viewport
    }

    /// Mints the one write handle for a slice cell.
    ///
    /// # Errors
    ///
    /// Returns [`SlotTaken`] if the slot is already registered —
    /// double activation is a wiring bug.
    pub fn register_cell<T: Send + Sync + 'static>(
        &self,
        key: SlotKey,
        initial: T,
    ) -> Result<crate::cell::TypedCell<T>, SlotTaken> {
        self.slices.register(key, initial)
    }

    /// Spawns a trouper service actor at `path` with typed
    /// subscriptions. Subscribe is the readiness point: topic cursors
    /// register synchronously inside `start`, so publishes after this
    /// call cannot be missed.
    ///
    /// `build` constructs the actor instance, capturing whatever cell
    /// handles and channels it needs (they cannot ride trouper's JSON
    /// args).
    pub fn spawn_service<A, T, E>(&self, path: ActorPath, build: T) -> ActorPath
    where
        A: ServiceActor + Send + Sync + 'static,
        T: FnOnce() -> Result<A, E> + Send + 'static,
        E: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        trouper::builder::spawn_service_builder::<A>(self.system)
            .at(path)
            .start_with(move || {
                Box::pin(async move {
                    build().map_err(|err| {
                        error_stack::Report::new(SpawnError)
                            .attach(err.into())
                            .change_context(trouper::registry::RegistryError::InvalidSpec)
                    })
                })
            })
            .start()
    }

    /// Attaches route rows to the key-route table.
    pub fn attach_rows<R>(&self, rows: R)
    where
        R: IntoIterator<Item = RouteRow>,
    {
        for row in rows {
            self.key_routes.attach(row);
        }
    }

    /// Registers the synchronous input hook for a slice's scope: an
    /// editing-intent interceptor served while that scope is focused.
    /// Staged on the host; composition installs all hooks into
    /// [`KeyRoutes`] after activation.
    pub fn register_input_hook<F>(&mut self, scope: SliceScopeId, serve: F)
    where
        F: Fn(&EditIntent) -> Option<RouteResult> + Send + Sync + 'static,
    {
        self.hooks.register(scope, Arc::new(serve));
    }

    /// Declares a tab scope backed by a slot.
    pub fn register_tab_scope(&self, scope: SliceScopeId, slot: SlotKey) {
        self.slices.register_tab_scope(scope, slot);
    }

    /// Registers a slice overlay's geometry for its scope.
    pub fn register_overlay(&self, scope: SliceScopeId, overlay: crate::slices::OverlayFn) {
        self.slices.register_overlay(scope, overlay);
    }

    /// Declares the slot backing a scope's overlay content.
    pub fn register_overlay_slot(&self, scope: SliceScopeId, slot: SlotKey) {
        self.slices.register_overlay_slot(scope, slot);
    }

    /// Registers a scope's overlay renderer.
    /// Marks `scope`'s overlay rect as a selectable region (popups with
    /// focusable content).
    pub fn register_overlay_selectable(&self, scope: &SliceScopeId) {
        self.slices.register_overlay_selectable(scope);
    }

    pub fn register_overlay_view(&self, scope: SliceScopeId, view: OverlayViewFn<C>) {
        self.overlay_views.register(scope, view);
    }

    /// Sets the slice's feature flag in the registry (read model for
    /// route-action gates).
    pub fn set_flag(&self, slice: &str, enabled: bool) {
        self.slices.set_flag(slice, enabled);
    }

    /// Ends activation, installing the staged input hooks via `install`.
    ///
    /// A slice's `jinn.toml` section needs no staging: it reads the
    /// configuration layer at the point of use, so there is no
    /// fail-fast gate left to run here.
    pub fn finalize<I>(self, install: I)
    where
        I: FnMut(SliceScopeId, crate::route::InputHook),
    {
        self.hooks.install(install);
    }
}

/// The build-failure error for [`SliceHost::spawn_service`] overrides.
#[derive(Debug, wherror::Error)]
#[error(debug)]
pub struct SpawnError;
