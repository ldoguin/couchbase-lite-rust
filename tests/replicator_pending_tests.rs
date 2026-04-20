extern crate couchbase_lite;

#[cfg(feature = "enterprise")]
use self::couchbase_lite::*;
#[cfg(feature = "enterprise")]
use utils::{add_doc, check_callback_with_wait, default_collection};

#[cfg(feature = "enterprise")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "enterprise")]
use std::mem::ManuallyDrop;

pub mod utils;

// ── pending_document_ids / is_document_pending ───────────────────────────────
//
// The _2 variants require the collection to be listed in the replicator's
// `collections` config.  The tester uses the legacy database-level config, so
// we use the non-deprecated (non-_2) variants here.

// Before replication starts every locally-written document must appear in
// pending_document_ids.  After a successful push the set must be empty.
#[test]
#[cfg(feature = "enterprise")]
#[allow(deprecated)]
fn pending_document_ids_before_and_after_push() {
    let config = utils::ReplicationTestConfiguration {
        replicator_type: ReplicatorType::Push,
        continuous: false,
        ..Default::default()
    };
    let mut tester = utils::ReplicationTwoDbsTester::new(
        config,
        Box::new(ReplicationConfigurationContext::default()),
    );
    tester.test(|local_db, _central_db, repl| {
        add_doc(local_db, "pending-1", 1, "one");
        add_doc(local_db, "pending-2", 2, "two");

        let pending = repl.pending_document_ids().expect("pending_document_ids");
        assert!(
            pending.contains("pending-1"),
            "pending-1 not in set: {pending:?}"
        );
        assert!(
            pending.contains("pending-2"),
            "pending-2 not in set: {pending:?}"
        );

        assert!(
            repl.is_document_pending("pending-1")
                .expect("is_document_pending"),
            "pending-1 should be pending"
        );
        assert!(
            repl.is_document_pending("pending-2")
                .expect("is_document_pending"),
            "pending-2 should be pending"
        );
        assert!(
            !repl
                .is_document_pending("nonexistent")
                .expect("is_document_pending"),
            "nonexistent should not be pending"
        );

        repl.start(false);
        assert!(
            check_callback_with_wait(
                || repl.status().activity == ReplicatorActivityLevel::Stopped,
                Some(10)
            ),
            "replicator did not stop after push"
        );

        let after = repl.pending_document_ids().expect("pending after push");
        assert!(
            after.is_empty(),
            "pending set should be empty after push, got: {after:?}"
        );
    });
}

// is_document_pending returns false for a document that has already been
// pushed, and true for one that hasn't been pushed yet.
#[test]
#[cfg(feature = "enterprise")]
#[allow(deprecated)]
fn is_document_pending_reflects_push_state() {
    let config = utils::ReplicationTestConfiguration {
        replicator_type: ReplicatorType::Push,
        continuous: false,
        ..Default::default()
    };
    let mut tester = utils::ReplicationTwoDbsTester::new(
        config,
        Box::new(ReplicationConfigurationContext::default()),
    );
    tester.test(|local_db, _central_db, repl| {
        add_doc(local_db, "pushed-doc", 42, "hello");

        assert!(
            repl.is_document_pending("pushed-doc")
                .expect("is_document_pending"),
            "should be pending before push"
        );

        repl.start(false);
        assert!(
            check_callback_with_wait(
                || repl.status().activity == ReplicatorActivityLevel::Stopped,
                Some(10)
            ),
            "replicator did not stop"
        );

        assert!(
            !repl
                .is_document_pending("pushed-doc")
                .expect("is_document_pending"),
            "should not be pending after push"
        );
    });
}

// ── add_document_listener ─────────────────────────────────────────────────────

// add_document_listener consumes Replicator (builder pattern).  Inside
// tester.test() we have &mut Replicator.  We use ManuallyDrop + ptr::read/write
// to take ownership without running the destructor on the placeholder, attach
// the listener, then write the result back.
#[cfg(feature = "enterprise")]
fn attach_doc_listener(repl: &mut Replicator, listener: ReplicatedDocumentListener) {
    // SAFETY: we immediately write a valid Replicator back before returning.
    // ManuallyDrop prevents the destructor from running on the moved-out value.
    unsafe {
        let old = ManuallyDrop::new(std::ptr::read(repl as *const Replicator));
        let new = ManuallyDrop::into_inner(old).add_document_listener(listener);
        std::ptr::write(repl as *mut Replicator, new);
    }
}

// add_document_listener fires for each document that is pushed, reporting
// Direction::Pushed and the correct document IDs.
#[test]
#[cfg(feature = "enterprise")]
fn document_listener_fires_on_push() {
    let pushed_ids: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let pushed_ids_clone = Arc::clone(&pushed_ids);

    let config = utils::ReplicationTestConfiguration {
        replicator_type: ReplicatorType::Push,
        continuous: true,
        ..Default::default()
    };
    let mut tester = utils::ReplicationTwoDbsTester::new(
        config,
        Box::new(ReplicationConfigurationContext::default()),
    );

    tester.test(|local_db, central_db, repl| {
        let ids = Arc::clone(&pushed_ids_clone);
        attach_doc_listener(
            repl,
            Box::new(move |direction, docs| {
                if matches!(direction, Direction::Pushed) {
                    let mut g = ids.lock().unwrap();
                    for doc in docs {
                        g.push(doc.id.clone());
                    }
                }
            }),
        );
        repl.start(false);

        add_doc(local_db, "ldoc-1", 1, "one");
        add_doc(local_db, "ldoc-2", 2, "two");

        // Wait until both docs appear in the central DB.
        assert!(
            check_callback_with_wait(
                || default_collection(central_db)
                    .get_document("ldoc-1")
                    .is_ok()
                    && default_collection(central_db)
                        .get_document("ldoc-2")
                        .is_ok(),
                Some(10)
            ),
            "docs did not replicate to central"
        );
        // Wait for the listener to record both IDs.
        assert!(
            check_callback_with_wait(|| pushed_ids.lock().unwrap().len() >= 2, Some(5)),
            "document listener did not fire for both docs"
        );

        let ids = pushed_ids.lock().unwrap().clone();
        assert!(
            ids.contains(&"ldoc-1".to_string()),
            "ldoc-1 not in pushed IDs: {ids:?}"
        );
        assert!(
            ids.contains(&"ldoc-2".to_string()),
            "ldoc-2 not in pushed IDs: {ids:?}"
        );
    });
}

// add_document_listener also fires for pulled documents, reporting
// Direction::Pulled.
#[test]
#[cfg(feature = "enterprise")]
fn document_listener_fires_on_pull() {
    let pulled_ids: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let pulled_ids_clone = Arc::clone(&pulled_ids);

    let config = utils::ReplicationTestConfiguration {
        replicator_type: ReplicatorType::Pull,
        continuous: true,
        ..Default::default()
    };
    let mut tester = utils::ReplicationTwoDbsTester::new(
        config,
        Box::new(ReplicationConfigurationContext::default()),
    );

    tester.test(|_local_db, central_db, repl| {
        let ids = Arc::clone(&pulled_ids_clone);
        attach_doc_listener(
            repl,
            Box::new(move |direction, docs| {
                if matches!(direction, Direction::Pulled) {
                    let mut g = ids.lock().unwrap();
                    for doc in docs {
                        g.push(doc.id.clone());
                    }
                }
            }),
        );
        repl.start(false);

        // Write docs to the central DB — the pull replicator will fetch them.
        add_doc(central_db, "rdoc-1", 10, "ten");
        add_doc(central_db, "rdoc-2", 20, "twenty");

        assert!(
            check_callback_with_wait(|| pulled_ids.lock().unwrap().len() >= 2, Some(10)),
            "document listener did not fire for pulled docs"
        );

        let ids = pulled_ids.lock().unwrap().clone();
        assert!(
            ids.contains(&"rdoc-1".to_string()),
            "rdoc-1 not in pulled IDs: {ids:?}"
        );
        assert!(
            ids.contains(&"rdoc-2".to_string()),
            "rdoc-2 not in pulled IDs: {ids:?}"
        );
    });
}
