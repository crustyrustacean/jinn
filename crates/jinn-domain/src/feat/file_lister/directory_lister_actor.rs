//! [`DirectoryListerActor`] — async directory listing for the `@path` popup.

use error_stack::Report;
use jinn_chat_input_msg::{FileEntry, ListDirectory};
use trouper::actor::{ActorPath, MsgHandler, ServiceActor};
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;

use crate::common::actor_deps::{ActorDeps, BusPublish};
use crate::common::services::bus_service::BusService;
use crate::common::state::State;

/// Dependencies for [`DirectoryListerActor`].
#[derive(Clone)]
pub struct DirectoryListerActorDeps {
    /// Runtime services and bus access.
    pub deps: ActorDeps,
    /// Shared application state.
    pub state: State,
}

/// Lists directories on `ListDirectory` commands and writes results to
/// `frontend.file_picker`.
pub struct DirectoryListerActor {
    /// Bus service.
    bus: BusService,
    /// Shared application state.
    state: State,
}

impl BusPublish for DirectoryListerActor {
    fn bus(&self) -> &BusService {
        &self.bus
    }
}

impl ServiceActor for DirectoryListerActor {
    #[expect(
        clippy::unused_async_trait_impl,
        reason = "ServiceActor::start is async by trait contract"
    )]
    async fn start(_args: &trouper::json::Json) -> Result<Self, Report<RegistryError>> {
        // Never called: spawned via `spawn`'s start_with (typed deps can't
        // ride the JSON args).
        Err(Report::new(RegistryError::InvalidSpec)
            .attach("DirectoryListerActor spawns via start_with"))
    }
}

/// Static path the lister spawns at (one instance per process).
pub const DIRECTORY_LISTER_PATH: &str = "jinn.file_lister.actor";

impl DirectoryListerActor {
    /// Spawns the lister onto the trouper system; its subscription is
    /// live when this returns.
    ///
    /// # Panics
    ///
    /// Panics if the actor's path is already taken or its topic
    /// subscription fails — both mean a wiring bug at composition.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "port convention: spawn takes owned deps and clones into start_with"
    )]
    pub fn spawn(
        system: &trouper::system::ActorSystem,
        deps: DirectoryListerActorDeps,
    ) -> ActorPath {
        let path = ActorPath::new(DIRECTORY_LISTER_PATH);
        trouper::builder::spawn_service_builder::<Self>(system)
            .at(path.clone())
            .start_with({
                let deps = deps.clone();
                move || {
                    let deps = deps.clone();
                    Box::pin(async move {
                        Ok(Self {
                            bus: deps.deps.services.bus.clone(),
                            state: deps.state,
                        })
                    })
                }
            })
            .handles::<ListDirectory>()
            .mailbox(64, trouper::inbox::OverloadPolicy::Block)
            .start();
        path
    }
}

impl MsgHandler<ListDirectory> for DirectoryListerActor {
    async fn handle(&mut self, msg: &ListDirectory, _ctx: &mut MsgCtx<'_>) {
        let path = msg.path.clone();
        let request_id = msg.request_id;
        let result = tokio::task::spawn_blocking(move || list_dir_blocking(&path)).await;
        let entries = result.unwrap_or_default();

        // Staleness guard: write only if this reply is still the expected one.
        self.state.with_file_picker(|ops| {
            let picker = ops.file_picker();
            if picker.expected_request_id == request_id {
                picker.entries = entries;
                picker.loading = false;
            }
        });
    }
}

/// Reads a directory on a blocking thread. On any error (missing dir,
/// permission denied), returns an empty list — the popup shows `<empty>`.
fn list_dir_blocking(path: &std::path::Path) -> Vec<FileEntry> {
    let read = std::fs::read_dir(path);
    let Ok(read) = read else {
        return Vec::new();
    };
    let mut entries: Vec<FileEntry> = read
        .filter_map(std::result::Result::ok)
        .map(|entry| {
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            FileEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir,
            }
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}
