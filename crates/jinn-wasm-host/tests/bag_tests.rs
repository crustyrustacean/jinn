use jinn_core_types::{PluginInstanceId, SessionId};

use jinn_wasm_host::bag::{GlobalBagStore, InstanceBagStore};

fn sid() -> SessionId {
    SessionId::new()
}

fn iid() -> PluginInstanceId {
    PluginInstanceId::new()
}

#[test]
fn instance_bag_roundtrips_bytes_for_session_key() {
    // Given an empty per-instance bag store.
    let store = InstanceBagStore::new();
    let session = sid();
    let instance = iid();

    // When writing then reading.
    store.set_for_session(&session, &instance, b"hello world".to_vec());
    let got = store.get_for_session(&session, &instance);

    // Then the bytes round-trip exactly.
    assert_eq!(got.as_deref(), Some(b"hello world".as_slice()));
}

#[test]
fn instance_bag_isolates_different_instances() {
    // Given a store with one instance's bag.
    let store = InstanceBagStore::new();
    let session = sid();
    let a = iid();
    let b = iid();
    store.set_for_session(&session, &a, b"alice".to_vec());

    // When reading instance b (different).
    let got = store.get_for_session(&session, &b);

    // Then it is empty — slots are isolated by instance id.
    assert!(got.is_none());
}

#[test]
fn instance_bag_overwrites_on_set() {
    // Given a bag with an initial value.
    let store = InstanceBagStore::new();
    let session = sid();
    let instance = iid();
    store.set_for_session(&session, &instance, b"v1".to_vec());

    // When overwriting.
    store.set_for_session(&session, &instance, b"v2".to_vec());

    // Then only the latest value is present.
    assert_eq!(
        store.get_for_session(&session, &instance).as_deref(),
        Some(b"v2".as_slice())
    );
}

#[test]
fn global_plugin_bag_roundtrips() {
    // Given an empty per-instance store.
    let store = InstanceBagStore::new();

    // When writing a global-plugin bag.
    store.set("welcome", b"greeted".to_vec());

    // Then it is retrievable by plugin name.
    assert_eq!(store.get("welcome").as_deref(), Some(b"greeted".as_slice()));
}

#[test]
fn bag_shares_state_across_clones() {
    // Given a store shared between two handles (sync store + async store).
    let async_store = InstanceBagStore::new();
    let sync_store = async_store.clone();
    let session = sid();
    let instance = iid();

    // When the async store writes.
    async_store.set_for_session(&session, &instance, b"from-async".to_vec());

    // Then the sync store observes the write (shared underlying DashMap).
    assert_eq!(
        sync_store.get_for_session(&session, &instance).as_deref(),
        Some(b"from-async".as_slice())
    );
}

#[test]
fn global_data_roundtrips_bytes() {
    // Given an empty global-data store.
    let globals = GlobalBagStore::new();

    // When writing under a key.
    globals.set("verdict:origin-1", b"pass".to_vec());

    // Then it is retrievable.
    assert_eq!(
        globals.get("verdict:origin-1").as_deref(),
        Some(b"pass".as_slice())
    );
}

#[test]
fn global_data_shares_across_clones() {
    // Given two handles to the same global store.
    let a = GlobalBagStore::new();
    let b = a.clone();

    // When a writes.
    a.set("count", b"3".to_vec());

    // Then b observes it.
    assert_eq!(b.get("count").as_deref(), Some(b"3".as_slice()));
}

#[test]
fn global_data_remove_returns_and_deletes() {
    // Given a store with a key.
    let globals = GlobalBagStore::new();
    globals.set("temp", b"x".to_vec());

    // When removing.
    let removed = globals.remove("temp");

    // Then the value is returned and the key is gone.
    assert_eq!(removed.as_deref(), Some(b"x".as_slice()));
    assert!(globals.get("temp").is_none());
}

#[test]
fn global_data_remove_missing_key_returns_none() {
    // Given an empty store.
    let globals = GlobalBagStore::new();

    // When removing a non-existent key.
    let removed = globals.remove("nope");

    // Then none is returned.
    assert!(removed.is_none());
}

#[test]
fn global_data_lists_all_keys() {
    // Given a store with multiple keys.
    let globals = GlobalBagStore::new();
    globals.set("verdict:1", b"a".to_vec());
    globals.set("verdict:2", b"b".to_vec());
    globals.set("other", b"c".to_vec());

    // When listing keys.
    let mut keys = globals.keys();
    keys.sort();

    // Then all keys are present.
    assert_eq!(keys, vec!["other", "verdict:1", "verdict:2"]);
}
