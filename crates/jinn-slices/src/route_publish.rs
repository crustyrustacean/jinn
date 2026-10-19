//! The publish surface a [`PublishClosure`](crate::PublishClosure)
//! drains against.
//!
//! Implemented by the kernel's bus wrapper (`BusService`); slice crates
//! only ever see this trait, so the fabric's subscriber registry and its
//! recording mode stay kernel-private while closures publish through
//! the exact same path as every other emitter.

/// The erased publish sink closures hand schema-tagged payloads to.
pub trait PublishSink {
    /// Publishes one schema-tagged message onto the fabric.
    ///
    /// `name` is the message's short type name (recording-mode parity);
    /// `payload` is the serde-serialized message body.
    ///
    /// The publish is *erased*: the transport comes from the schema's
    /// declared kind, not from the caller. An Event schema broadcasts to
    /// every actor that declared `.handles::<M>()`; a Command schema
    /// routes to one handler, rotating between them when a second actor
    /// declares the same schema. A message with one declarant behaves
    /// identically on this path and on a direct publish.
    fn publish_schema(
        &self,
        schema_id: trouper::schema::SchemaId,
        payload: serde_json::Value,
        name: &'static str,
    );
}
