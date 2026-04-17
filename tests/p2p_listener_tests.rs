extern crate couchbase_lite;
use self::couchbase_lite::*;
use std::collections::HashMap;
use std::time::Duration;
use std::thread;

pub mod utils;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Open a named database in the given temp directory.
fn open_db(name: &str, dir: &std::path::Path) -> Database {
    let cfg = DatabaseConfiguration {
        directory: dir,
        #[cfg(feature = "enterprise")]
        encryption_key: None,
    };
    Database::open(name, Some(cfg)).unwrap_or_else(|e| panic!("open {name}: {e}"))
}

/// Save a document `{key: value}` into the default collection of `db`.
fn put_doc(db: &Database, id: &str, key: &str, value: &str) {
    let mut doc = Document::new_with_id(id);
    doc.mutable_properties().at(key).put_string(value);
    db.default_collection_or_error()
        .unwrap()
        .save_document_with_concurency_control(&mut doc, ConcurrencyControl::FailOnConflict)
        .unwrap_or_else(|e| panic!("save {id}: {e}"));
}

/// Return `true` if the default collection of `db` contains a document with `id`.
fn has_doc(db: &Database, id: &str) -> bool {
    db.default_collection_or_error()
        .unwrap()
        .get_document(id)
        .is_ok()
}

/// Build a `ReplicatorConfiguration` that connects `peer` to `url` (the listener).
/// TLS is disabled so we can use plain `ws://`.
fn peer_config(peer: &Database, url: &str) -> ReplicatorConfiguration {
    let endpoint = Endpoint::new_with_url(url).unwrap_or_else(|e| panic!("endpoint {url}: {e}"));
    ReplicatorConfiguration {
        database: Some(peer.clone()),
        endpoint,
        replicator_type: ReplicatorType::PushAndPull,
        continuous: true,
        disable_auto_purge: true,
        max_attempts: 10,
        max_attempt_wait_time: 300,
        heartbeat: 30,
        authenticator: None,
        proxy: None,
        headers: HashMap::new(),
        pinned_server_certificate: None,
        trusted_root_certificates: None,
        channels: MutableArray::default(),
        document_ids: MutableArray::default(),
        collections: None,
        accept_parent_domain_cookies: false,
        #[cfg(feature = "enterprise")]
        accept_only_self_signed_server_certificate: false,
    }
}

fn default_context() -> Box<ReplicationConfigurationContext> {
    Box::new(ReplicationConfigurationContext::default())
}

/// Wait up to `secs` seconds for `predicate` to return true, polling every 100 ms.
fn wait_for(secs: u64, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < deadline {
        if predicate() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Two peers connect to a listener. A document written directly into the
/// listener database appears in both peers.
#[test]
#[cfg(feature = "enterprise")]
fn listener_doc_syncs_to_all_peers() {
    utils::init_logging();
    let tmp = tempdir::TempDir::new("p2p_test").unwrap();

    let listener_db = open_db("listener", tmp.path());
    let peer1_db = open_db("peer1", tmp.path());
    let peer2_db = open_db("peer2", tmp.path());

    // Start listener on an OS-assigned port.
    let coll = listener_db.default_collection_or_error().unwrap();
    let listener = UrlEndpointListener::new(ListenerConfiguration {
        collections: vec![coll],
        port: 0,
        tls_identity: None,
        authenticator: None,
        ..Default::default()
    })
    .expect("create listener");
    listener.start().expect("start listener");

    let url = format!("ws://127.0.0.1:{}/listener", listener.port());

    // Connect both peers.
    let mut rep1 = Replicator::new(peer_config(&peer1_db, &url), default_context()).unwrap();
    let mut rep2 = Replicator::new(peer_config(&peer2_db, &url), default_context()).unwrap();
    rep1.start(false);
    rep2.start(false);

    // Write a document directly into the listener database.
    put_doc(&listener_db, "from-listener", "source", "listener");

    // Both peers must receive it.
    assert!(
        wait_for(15, || has_doc(&peer1_db, "from-listener")),
        "peer1 did not receive 'from-listener'"
    );
    assert!(
        wait_for(15, || has_doc(&peer2_db, "from-listener")),
        "peer2 did not receive 'from-listener'"
    );

    rep1.stop(Some(5));
    rep2.stop(Some(5));
    listener.stop();
    listener_db.delete().unwrap();
    peer1_db.delete().unwrap();
    peer2_db.delete().unwrap();
}

/// A document written on peer1 reaches the listener and then peer2.
#[test]
#[cfg(feature = "enterprise")]
fn peer_doc_propagates_through_listener_to_other_peer() {
    utils::init_logging();
    let tmp = tempdir::TempDir::new("p2p_test").unwrap();

    let listener_db = open_db("listener", tmp.path());
    let peer1_db = open_db("peer1", tmp.path());
    let peer2_db = open_db("peer2", tmp.path());

    let coll = listener_db.default_collection_or_error().unwrap();
    let listener = UrlEndpointListener::new(ListenerConfiguration {
        collections: vec![coll],
        port: 0,
        tls_identity: None,
        authenticator: None,
        ..Default::default()
    })
    .expect("create listener");
    listener.start().expect("start listener");

    let url = format!("ws://127.0.0.1:{}/listener", listener.port());

    let mut rep1 = Replicator::new(peer_config(&peer1_db, &url), default_context()).unwrap();
    let mut rep2 = Replicator::new(peer_config(&peer2_db, &url), default_context()).unwrap();
    rep1.start(false);
    rep2.start(false);

    // Write on peer1.
    put_doc(&peer1_db, "from-peer1", "source", "peer1");

    // Must arrive at the listener first.
    assert!(
        wait_for(15, || has_doc(&listener_db, "from-peer1")),
        "listener did not receive 'from-peer1' from peer1"
    );

    // Then propagate to peer2.
    assert!(
        wait_for(15, || has_doc(&peer2_db, "from-peer1")),
        "peer2 did not receive 'from-peer1' from listener"
    );

    rep1.stop(Some(5));
    rep2.stop(Some(5));
    listener.stop();
    listener_db.delete().unwrap();
    peer1_db.delete().unwrap();
    peer2_db.delete().unwrap();
}

/// Documents written on each peer independently both reach the listener and
/// cross-replicate to the other peer.
#[test]
#[cfg(feature = "enterprise")]
fn docs_from_multiple_peers_cross_replicate() {
    utils::init_logging();
    let tmp = tempdir::TempDir::new("p2p_test").unwrap();

    let listener_db = open_db("listener", tmp.path());
    let peer1_db = open_db("peer1", tmp.path());
    let peer2_db = open_db("peer2", tmp.path());

    let coll = listener_db.default_collection_or_error().unwrap();
    let listener = UrlEndpointListener::new(ListenerConfiguration {
        collections: vec![coll],
        port: 0,
        tls_identity: None,
        authenticator: None,
        ..Default::default()
    })
    .expect("create listener");
    listener.start().expect("start listener");

    let url = format!("ws://127.0.0.1:{}/listener", listener.port());

    let mut rep1 = Replicator::new(peer_config(&peer1_db, &url), default_context()).unwrap();
    let mut rep2 = Replicator::new(peer_config(&peer2_db, &url), default_context()).unwrap();
    rep1.start(false);
    rep2.start(false);

    // Each peer writes its own document concurrently.
    put_doc(&peer1_db, "peer1-doc", "source", "peer1");
    put_doc(&peer2_db, "peer2-doc", "source", "peer2");

    // Both docs must reach the listener.
    assert!(
        wait_for(15, || has_doc(&listener_db, "peer1-doc")),
        "listener did not receive 'peer1-doc'"
    );
    assert!(
        wait_for(15, || has_doc(&listener_db, "peer2-doc")),
        "listener did not receive 'peer2-doc'"
    );

    // peer1 must receive peer2's doc and vice versa.
    assert!(
        wait_for(15, || has_doc(&peer1_db, "peer2-doc")),
        "peer1 did not receive 'peer2-doc'"
    );
    assert!(
        wait_for(15, || has_doc(&peer2_db, "peer1-doc")),
        "peer2 did not receive 'peer1-doc'"
    );

    rep1.stop(Some(5));
    rep2.stop(Some(5));
    listener.stop();
    listener_db.delete().unwrap();
    peer1_db.delete().unwrap();
    peer2_db.delete().unwrap();
}

/// The listener connection count reflects the number of connected peers.
#[test]
#[cfg(feature = "enterprise")]
fn listener_connection_count_tracks_peers() {
    utils::init_logging();
    let tmp = tempdir::TempDir::new("p2p_test").unwrap();

    let listener_db = open_db("listener", tmp.path());
    let peer1_db = open_db("peer1", tmp.path());
    let peer2_db = open_db("peer2", tmp.path());

    let coll = listener_db.default_collection_or_error().unwrap();
    let listener = UrlEndpointListener::new(ListenerConfiguration {
        collections: vec![coll],
        port: 0,
        tls_identity: None,
        authenticator: None,
        ..Default::default()
    })
    .expect("create listener");
    listener.start().expect("start listener");

    let url = format!("ws://127.0.0.1:{}/listener", listener.port());

    let mut rep1 = Replicator::new(peer_config(&peer1_db, &url), default_context()).unwrap();
    let mut rep2 = Replicator::new(peer_config(&peer2_db, &url), default_context()).unwrap();
    rep1.start(false);
    rep2.start(false);

    // Wait until both peers have connected (connection_count >= 2).
    assert!(
        wait_for(15, || listener.status().connection_count >= 2),
        "listener did not see 2 connections within timeout"
    );

    // Disconnect peer1 and verify the count drops.
    rep1.stop(Some(5));
    assert!(
        wait_for(15, || listener.status().connection_count < 2),
        "listener connection count did not drop after peer1 disconnected"
    );

    rep2.stop(Some(5));
    listener.stop();
    listener_db.delete().unwrap();
    peer1_db.delete().unwrap();
    peer2_db.delete().unwrap();
}
