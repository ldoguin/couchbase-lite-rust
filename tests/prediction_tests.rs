extern crate couchbase_lite;
use self::couchbase_lite::*;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

pub mod utils;

// ── register / unregister lifecycle ──────────────────────────────────────────

/// Registering and immediately unregistering a model must not panic or crash.
#[test]
#[cfg(feature = "enterprise")]
fn register_unregister_no_crash() {
    struct Noop;
    impl PredictiveModel for Noop {
        fn predict(&self, _input: fleece::Dict) -> Option<MutableDict> {
            None
        }
    }

    register_predictive_model("noop_model", Noop);
    unregister_predictive_model("noop_model");
}

/// Unregistering a name that was never registered must not panic.
#[test]
#[cfg(feature = "enterprise")]
fn unregister_unknown_name_no_crash() {
    unregister_predictive_model("does_not_exist_xyz");
}

// ── model is called during query execution ────────────────────────────────────

/// A registered model's `predict` method is invoked when a `PREDICTION()`
/// query runs, and the output dict is returned as the query result.
#[test]
#[cfg(feature = "enterprise")]
fn model_called_during_query() {
    let call_count = Arc::new(AtomicU32::new(0));

    struct Counter {
        count: Arc<AtomicU32>,
    }

    impl PredictiveModel for Counter {
        fn predict(&self, _input: fleece::Dict) -> Option<MutableDict> {
            self.count.fetch_add(1, Ordering::SeqCst);
            let mut out = MutableDict::new();
            out.at("result").put_i64(42);
            Some(out)
        }
    }

    let model_name = "counter_model";
    register_predictive_model(model_name, Counter { count: Arc::clone(&call_count) });

    utils::with_db(|db| {
        // Insert one document so the query has a row to process.
        let mut coll = db.default_collection_or_error().unwrap();
        let mut doc = Document::new_with_id("pred_doc");
        doc.set_properties_as_json(r#"{"x": 1}"#).unwrap();
        coll.save_document(&mut doc).unwrap();

        let query = Query::new(
            db,
            QueryLanguage::N1QL,
            "SELECT PREDICTION(counter_model, {\"x\": _.x}).result FROM _",
        )
        .expect("create prediction query");

        let mut results = query.execute().expect("execute");
        let row = results.next().expect("expected one result row");
        let value = row.get(0).as_i64();
        assert_eq!(
            value,
            Some(42),
            "expected model output 42, got {:?}",
            value
        );
    });

    assert!(
        call_count.load(Ordering::SeqCst) >= 1,
        "model predict() should have been called at least once"
    );

    unregister_predictive_model(model_name);
}

// ── model returning None produces NULL in query output ────────────────────────

/// When `predict` returns `None`, the `PREDICTION()` expression evaluates to
/// MISSING/NULL and the query row's value is absent.
#[test]
#[cfg(feature = "enterprise")]
fn model_returning_none_gives_null() {
    struct AlwaysNone;
    impl PredictiveModel for AlwaysNone {
        fn predict(&self, _input: fleece::Dict) -> Option<MutableDict> {
            None
        }
    }

    let model_name = "none_model";
    register_predictive_model(model_name, AlwaysNone);

    utils::with_db(|db| {
        let mut coll = db.default_collection_or_error().unwrap();
        let mut doc = Document::new_with_id("none_doc");
        doc.set_properties_as_json(r#"{"x": 1}"#).unwrap();
        coll.save_document(&mut doc).unwrap();

        let query = Query::new(
            db,
            QueryLanguage::N1QL,
            "SELECT PREDICTION(none_model, {\"x\": _.x}).result FROM _",
        )
        .expect("create query");

        let mut results = query.execute().expect("execute");
        let row = results.next().expect("expected one result row");
        // PREDICTION() returned None → the .result field is MISSING → as_i64() is None.
        assert!(
            row.get(0).as_i64().is_none(),
            "expected null/missing for None-returning model"
        );
    });

    unregister_predictive_model(model_name);
}

// ── model receives input fields ───────────────────────────────────────────────

/// The `input` dict passed to `predict` contains the fields from the query's
/// input expression.
#[test]
#[cfg(feature = "enterprise")]
fn model_receives_input_fields() {
    let received = Arc::new(std::sync::Mutex::new(None::<i64>));

    struct Inspector {
        received: Arc<std::sync::Mutex<Option<i64>>>,
    }

    impl PredictiveModel for Inspector {
        fn predict(&self, input: fleece::Dict) -> Option<MutableDict> {
            let val = input.get("x").as_i64();
            *self.received.lock().unwrap() = val;
            None
        }
    }

    let model_name = "inspector_model";
    register_predictive_model(
        model_name,
        Inspector { received: Arc::clone(&received) },
    );

    utils::with_db(|db| {
        let mut coll = db.default_collection_or_error().unwrap();
        let mut doc = Document::new_with_id("insp_doc");
        doc.set_properties_as_json(r#"{"x": 99}"#).unwrap();
        coll.save_document(&mut doc).unwrap();

        let query = Query::new(
            db,
            QueryLanguage::N1QL,
            "SELECT PREDICTION(inspector_model, {\"x\": _.x}) FROM _",
        )
        .expect("create query");

        query.execute().expect("execute").for_each(|_| {});
    });

    let got = *received.lock().unwrap();
    assert_eq!(got, Some(99), "model should have received x=99, got {:?}", got);

    unregister_predictive_model(model_name);
}
