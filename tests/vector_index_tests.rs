extern crate couchbase_lite;
use self::couchbase_lite::*;

pub mod utils;

/// `IndexUpdater` implements `Iterator<Item = Value>` — iterating yields one
/// `Value` per pending entry, and the count matches `updater.pending_count()`.
///
/// This test does not require the vector search extension: it only exercises
/// the iterator protocol on the `IndexUpdater` struct itself.
#[test]
#[cfg(feature = "enterprise")]
fn index_updater_iterator() {
    let ext_path = std::env::var("CBLITE_VECTOR_SEARCH_PATH").unwrap_or_default();
    match enable_vector_search(&ext_path) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("SKIP: vector search extension not available: {e}");
            return;
        }
    }

    utils::with_db(|db| {
        let mut coll = db.default_collection_or_error().unwrap();

        for i in 0u32..4 {
            let mut doc = Document::new_with_id(&format!("iter_{i}"));
            let mut props = doc.mutable_properties();
            props.at("v").put_i64(i as i64);
            coll.save_document(&mut doc).unwrap();
        }

        let config = VectorIndexConfiguration {
            lazy: true,
            ..VectorIndexConfiguration::new("v", 2, 2)
        };
        coll.create_vector_index("iter_idx", &config)
            .expect("create vector index");

        let index = coll.get_index("iter_idx").expect("get index");
        let updater = index
            .begin_update(10)
            .expect("begin_update")
            .expect("updater should be Some");

        // Verify ExactSizeIterator reports the correct length.
        assert_eq!(updater.len(), 4, "expected 4 pending entries");

        // Consume via Iterator — each item should be a valid (non-null) Value.
        let mut seen = 0usize;
        for val in updater {
            // The value is the integer stored in "v"; it must be a valid Fleece value.
            assert!(
                val.as_i64().is_some(),
                "expected integer Value from iterator, got type {:?}",
                val.get_type()
            );
            seen += 1;
        }
        assert_eq!(seen, 4, "iterator should yield exactly 4 values");
    });
}

/// Full vector index lifecycle:
///   enable_vector_search (empty path = built-in) →
///   create lazy vector index →
///   insert documents →
///   IndexUpdater: set vectors → finish →
///   APPROX_VECTOR_DISTANCE query returns the nearest neighbour.
///
/// Uses a lazy index so we supply float vectors explicitly via `IndexUpdater`
/// rather than requiring documents to store float arrays.
#[test]
#[cfg(feature = "enterprise")]
fn vector_index_create_and_query() {
    // Enable the vector search extension. The extension path can be overridden
    // via the CBLITE_VECTOR_SEARCH_PATH env var; defaults to the current dir.
    // If the extension library is absent, skip rather than fail.
    let ext_path = std::env::var("CBLITE_VECTOR_SEARCH_PATH").unwrap_or_default();
    match enable_vector_search(&ext_path) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("SKIP: vector search extension not available: {e}");
            return;
        }
    }

    utils::with_db(|db| {
        let mut coll = db.default_collection_or_error().unwrap();

        // Insert 3 documents. The lazy index expression is "label" (an integer
        // field); IndexUpdater will supply the actual float vectors below.
        for i in 0u32..3 {
            let mut doc = Document::new_with_id(&format!("vec_{i}"));
            let mut props = doc.mutable_properties();
            props.at("label").put_i64(i as i64);
            coll.save_document(&mut doc).unwrap();
        }

        // Create a lazy 4-dimensional vector index with 2 centroids.
        let config = VectorIndexConfiguration {
            lazy: true,
            ..VectorIndexConfiguration::new("label", 4, 2)
        };
        coll.create_vector_index("vec_idx", &config)
            .expect("create vector index");

        // Obtain the index and open an update pass (limit = 10).
        let index = coll.get_index("vec_idx").expect("get index");
        let updater = index
            .begin_update(10)
            .expect("begin_update")
            .expect("updater should be Some — index has 3 pending entries");

        assert_eq!(updater.pending_count(), 3, "expected 3 pending vectors");

        // Supply one 4-D unit vector per document.
        let vectors: &[&[f32]] = &[
            &[1.0, 0.0, 0.0, 0.0],
            &[0.0, 1.0, 0.0, 0.0],
            &[0.0, 0.0, 1.0, 0.0],
        ];
        for (i, v) in vectors.iter().enumerate() {
            updater.set_vector(i, v).expect("set_vector");
        }
        updater.finish().expect("finish");

        // Query: nearest neighbour to [1,0,0,0] should be vec_0.
        let query = Query::new(
            db,
            QueryLanguage::N1QL,
            "SELECT META().id FROM _default._default \
             ORDER BY APPROX_VECTOR_DISTANCE(label, [1,0,0,0]) LIMIT 1",
        )
        .expect("create query");

        let mut results = query.execute().expect("execute query");
        let row = results.next().expect("expected at least one result row");
        let id = row.get(0).as_string().unwrap_or_default().to_string();
        assert_eq!(
            id, "vec_0",
            "nearest neighbour to [1,0,0,0] should be vec_0"
        );
    });
}
