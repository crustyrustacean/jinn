//! Trouper → kameo reverse relays: one relay actor per route.
//!
//! A slice declares a reverse route at activation
//! ([`SliceHost::reverse`]) when a trouper-side event must reach
//! pre-port kameo consumers. There is no central table: each route is
//! a [`ServiceActor`] at path `trouper-to-kameo/<schema-name>`, whose
//! handler republishes onto the kameo bus. Topic subscription happens
//! at spawn — subscribe is the readiness point, so publishes after
//! [`spawn_reverse_relay`] returns cannot be missed.

use trouper::actor::MsgHandler;
use trouper::actor::ServiceActor;
use trouper::context::MsgCtx;
use trouper::registry::RegistryError;
use trouper::system::ActorSystem;
use trouper::types::ActorPath;

use crate::common::services::bus_service::BusService;
use jinn_slices::host::ReverseMessage;

/// One reverse relay for message `M` on one topic.
pub struct ReverseRelay<M> {
    bus: BusService,
    _marker: std::marker::PhantomData<fn() -> M>,
}

impl<M> ServiceActor for ReverseRelay<M>
where
    M: ReverseMessage + crate::common::bus::BusMessage,
{
    async fn start(
        _args: &serde_json::Value,
    ) -> Result<Self, trouper::error_stack::Report<RegistryError>> {
        Err(
            trouper::error_stack::IntoReport::into_report(RegistryError::InvalidSpec)
                .attach("ReverseRelay spawns via start_with; start requires the bus handle"),
        )
    }
}

impl<M> MsgHandler<M> for ReverseRelay<M>
where
    M: ReverseMessage + crate::common::bus::BusMessage,
{
    async fn handle(&mut self, msg: M, _ctx: &mut MsgCtx<'_>) {
        self.bus.publish(msg).await;
    }
}

/// Spawns the relay actor for `M` at `trouper-to-kameo/<name>` and
/// subscribes it to its schema-named topic. Live on return.
///
/// # Panics
///
/// Panics when the topic subscription fails — a broken trouper
/// registry must abort wiring, not run unrouted.
#[expect(
    clippy::expect_used,
    reason = "a failed reverse-route subscription is a wiring bug that must abort launch"
)]
pub fn spawn_reverse_relay<M>(system: &std::sync::Arc<ActorSystem>, bus: BusService) -> ActorPath
where
    M: ReverseMessage + crate::common::bus::BusMessage,
{
    let name = M::schema_def().name;
    let path = ActorPath::new(format!("trouper-to-kameo/{name}"));
    let spawned = trouper::builder::spawn_service_builder::<ReverseRelay<M>>(system)
        .at(path.clone())
        .start_with(move || {
            Box::pin(async move {
                Ok(ReverseRelay::<M> {
                    bus,
                    _marker: std::marker::PhantomData,
                })
            })
        })
        .handles::<M>()
        .start();
    let topic = trouper::types::Topic::new(name.as_str());
    system
        .subscribe(&spawned, &topic, None)
        .expect("reverse relay subscribes its own schema topic");
    spawned
}
