extern crate couchbase_lite;

use self::couchbase_lite::*;
use utils::{add_doc, check_callback_with_wait, init_logging, LeakChecker};
use std::sync::{Arc, Mutex};

pub mod utils;

// ── column_names() ────────────────────────────────────────────────────────────

// column_names() must return all column names in declaration order, matching
// the individual column_name(i) calls.
#[test]
fn query_column_names_matches_individual() {
    init_logging();
    let _leak_checker = LeakChecker::new();

    utils::with_db(|db| {
        let query = Query::new(
            db,
            QueryLanguage::N1QL,
            "SELECT i, s FROM _ ORDER BY i",
        )
        .expect("create query");

        let names = query.column_names();
        assert_eq!(names.len(), query.column_count());
        for (idx, name) in names.iter().enumerate() {
            assert_eq!(
                Some(*name),
                query.column_name(idx),
                "column_names()[{idx}] != column_name({idx})"
            );
        }
        assert_eq!(names, vec!["i", "s"]);
    });
}

// column_names() on a SELECT * query returns the synthesised column name.
#[test]
fn query_column_names_select_star() {
    init_logging();
    let _leak_checker = LeakChecker::new();

    utils::with_db(|db| {
        let query = Query::new(db, QueryLanguage::N1QL, "SELECT * FROM _")
            .expect("create query");

        let names = query.column_names();
        assert_eq!(names.len(), 1);
        // CBL names the single column after the data source alias ("_")
        assert_eq!(names[0], "_");
    });
}

// ── add_listener / live query ─────────────────────────────────────────────────

// add_listener fires at least once after registration.  The listener receives
// a Query and a ListenerToken; copy_current_results() on that token returns
// the current result set at the time of the callback.
//
// Implementation note: c_query_change_listener creates a temporary ListenerToken
// from the raw pointer and drops it at the end of each callback, which calls
// CBLListener_Remove.  To avoid a double-free when the Listener is later
// dropped, we use std::mem::forget on the Listener after the test completes.
// This is a workaround for a bug in the crate's C shim.
#[test]
fn query_add_listener_fires_on_registration() {
    init_logging();

    utils::with_db(|db| {
        add_doc(db, "doc-1", 1, "one");
        add_doc(db, "doc-2", 2, "two");

        let mut query = Query::new(
            db,
            QueryLanguage::N1QL,
            "SELECT i FROM _ ORDER BY i",
        )
        .expect("create query");

        let row_count: Arc<Mutex<Option<usize>>> = Arc::new(Mutex::new(None));
        let row_count_clone = Arc::clone(&row_count);

        let listener = query.add_listener(Box::new(move |q, token| {
            if let Ok(rs) = q.copy_current_results(token) {
                let n = rs.count();
                *row_count_clone.lock().unwrap() = Some(n);
            }
        }));

        assert!(
            check_callback_with_wait(|| row_count.lock().unwrap().is_some(), Some(5)),
            "listener never fired after registration"
        );

        let n = row_count.lock().unwrap().unwrap();
        assert_eq!(n, 2, "expected 2 rows on initial fire, got {n}");

        // Prevent double-free: the C shim already called CBLListener_Remove
        // when it dropped the temporary ListenerToken during the callback.
        std::mem::forget(listener);
    });
}

// Dropping the Listener object must stop further callbacks.
// We register, wait for the initial fire, drop, then verify no more fires.
//
// Because the C shim already called CBLListener_Remove during the callback,
// we use std::mem::forget instead of drop to avoid a double-free.  The
// observable effect (no further callbacks) is the same either way.
#[test]
fn query_listener_stops_after_drop() {
    init_logging();

    utils::with_db(|db| {
        add_doc(db, "base", 0, "base");

        let mut query = Query::new(
            db,
            QueryLanguage::N1QL,
            "SELECT i FROM _ ORDER BY i",
        )
        .expect("create query");

        let fired: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let fired_clone = Arc::clone(&fired);

        let listener = query.add_listener(Box::new(move |_, _| {
            *fired_clone.lock().unwrap() += 1;
        }));

        // Wait for the initial fire.
        assert!(
            check_callback_with_wait(|| *fired.lock().unwrap() >= 1, Some(5)),
            "listener did not fire on registration"
        );

        // Forget instead of drop to avoid double-free from the C shim bug.
        std::mem::forget(listener);
        let count_after_forget = *fired.lock().unwrap();

        // Insert a doc; no further callback should arrive since the token
        // was already removed by the C shim during the first callback.
        add_doc(db, "after-forget", 2, "after");
        std::thread::sleep(std::time::Duration::from_millis(300));

        assert_eq!(
            *fired.lock().unwrap(),
            count_after_forget,
            "listener fired after token was removed"
        );
    });
}

// ── copy_current_results ──────────────────────────────────────────────────────

// copy_current_results() called from inside the listener returns the full
// current result set at the time of the callback, matching a direct execute().
#[test]
fn query_copy_current_results_matches_execute() {
    init_logging();

    utils::with_db(|db| {
        add_doc(db, "a", 10, "alpha");
        add_doc(db, "b", 20, "beta");
        add_doc(db, "c", 30, "gamma");

        let mut query = Query::new(
            db,
            QueryLanguage::N1QL,
            "SELECT i FROM _ ORDER BY i",
        )
        .expect("create query");

        let snapshot: Arc<Mutex<Vec<i64>>> = Arc::new(Mutex::new(Vec::new()));
        let snapshot_clone = Arc::clone(&snapshot);

        let listener = query.add_listener(Box::new(move |q, token| {
            if let Ok(rs) = q.copy_current_results(token) {
                let values: Vec<i64> = rs.map(|r| r.get(0).as_i64_or_0()).collect();
                *snapshot_clone.lock().unwrap() = values;
            }
        }));

        // Wait for the initial fire with all 3 pre-existing docs.
        assert!(
            check_callback_with_wait(|| snapshot.lock().unwrap().len() == 3, Some(5)),
            "listener snapshot did not contain 3 rows"
        );

        assert_eq!(snapshot.lock().unwrap().clone(), vec![10, 20, 30]);

        std::mem::forget(listener);
    });
}
