extern crate couchbase_lite;

use self::couchbase_lite::*;
use utils::{LeakChecker, default_collection, init_logging};
use std::io::{Read, Write};

pub mod utils;

// ── from_value / retain fix ───────────────────────────────────────────────────

// Attach a blob to a document, save it, then retrieve the blob via as_blob() on
// a freshly-loaded document.  The retrieved Blob must remain valid after the
// Document it came from is dropped — this exercises the retain() call added to
// Blob::from_value(), which prevents a use-after-free when Drop::drop releases
// the pointer.
#[test]
fn blob_from_value_retains_independently_of_document() {
    utils::with_db(|db| {
        let data = b"hello blob";
        let content_type = "text/plain";

        {
            let mut blob = Blob::new_from_data(data, content_type);
            let mut doc = Document::new_with_id("blob_doc");
            doc.mutable_properties().at("attachment").put_blob(&mut blob);
            default_collection(db)
                .save_document_with_concurency_control(&mut doc, ConcurrencyControl::FailOnConflict)
                .expect("save");
        }

        // Extract the blob then drop the document; blob must keep its own retain.
        let blob = {
            let doc = default_collection(db)
                .get_document("blob_doc")
                .expect("get_document");
            doc.properties()
                .get("attachment")
                .as_blob()
                .expect("property should be a blob")
        };

        assert_eq!(blob.content_type(), Some(content_type));
        assert_eq!(blob.length(), data.len() as u64);
        assert_eq!(blob.load_content().expect("load_content"), data);
    });
}

// Cloning a Blob increments the refcount; both the original and the clone must
// be independently usable and must not double-free on drop.
#[test]
fn blob_clone_is_independent() {
    init_logging();
    let _leak_checker = LeakChecker::new();

    utils::with_db(|db| {
        let data = b"clone me";

        let mut blob = Blob::new_from_data(data, "application/octet-stream");
        let mut doc = Document::new_with_id("clone_doc");
        doc.mutable_properties().at("b").put_blob(&mut blob);
        default_collection(db)
            .save_document_with_concurency_control(&mut doc, ConcurrencyControl::FailOnConflict)
            .expect("save");

        let doc = default_collection(db)
            .get_document("clone_doc")
            .expect("get_document");
        let original = doc.properties().get("b").as_blob().expect("blob");
        let cloned = original.clone();
        drop(original);

        assert_eq!(cloned.length(), data.len() as u64);
        assert_eq!(cloned.load_content().expect("load_content"), data);
    });
}

// as_blob() on a non-blob value must return None without panicking.
#[test]
fn blob_from_value_returns_none_for_non_blob() {
    init_logging();
    let _leak_checker = LeakChecker::new();

    let mut doc = Document::new_with_id("no_blob");
    doc.mutable_properties().at("x").put_i64(42);
    assert!(doc.properties().get("x").as_blob().is_none());
}

// ── accessors ─────────────────────────────────────────────────────────────────

// digest() returns a non-empty base64 SHA-1 string; properties() exposes the
// standard blob metadata keys (@type, digest, length, content_type).
#[test]
fn blob_digest_and_properties() {
    init_logging();
    let _leak_checker = LeakChecker::new();

    let data = b"digest me";
    let blob = Blob::new_from_data(data, "text/plain");

    let digest = blob.digest();
    assert!(!digest.is_empty(), "digest must not be empty");
    // CBL blob digests are prefixed "sha1-" followed by base64-encoded SHA-1
    assert!(
        digest.starts_with("sha1-"),
        "digest should start with 'sha1-': {digest}"
    );

    let props = blob.properties();
    assert_eq!(
        props.get("@type").as_string(),
        Some("blob"),
        "missing @type marker"
    );
    assert_eq!(
        props.get("length").as_i64_or_0(),
        data.len() as i64,
        "length mismatch in properties"
    );
    assert_eq!(
        props.get("content_type").as_string(),
        Some("text/plain"),
        "content_type mismatch in properties"
    );
    assert_eq!(
        props.get("digest").as_string(),
        Some(digest),
        "digest in properties must match digest()"
    );
}

// ── BlobWriter / new_from_stream ──────────────────────────────────────────────

// Write data through BlobWriter, create a blob from the stream, attach it to a
// document, save, reload, and verify the content round-trips correctly.
#[test]
fn blob_writer_round_trip() {
    utils::with_db(|db| {
        let data = b"written via stream";
        let content_type = "application/octet-stream";

        let blob = {
            let mut writer = BlobWriter::new(db).expect("BlobWriter::new");
            writer.write_all(data).expect("write_all");
            Blob::new_from_stream(writer, content_type)
        };

        assert_eq!(blob.length(), data.len() as u64);
        assert_eq!(blob.content_type(), Some(content_type));

        let mut doc = Document::new_with_id("stream_doc");
        doc.mutable_properties()
            .at("payload")
            .put_blob(&mut blob.clone());
        default_collection(db)
            .save_document_with_concurency_control(&mut doc, ConcurrencyControl::FailOnConflict)
            .expect("save");

        let reloaded = default_collection(db)
            .get_document("stream_doc")
            .expect("get_document");
        let stored = reloaded
            .properties()
            .get("payload")
            .as_blob()
            .expect("blob property");

        assert_eq!(stored.load_content().expect("load_content"), data);
    });
}

// Write data in multiple chunks and verify the assembled content is correct.
// The blob must be saved in a document before load_content() can read it back.
#[test]
fn blob_writer_multi_chunk() {
    utils::with_db(|db| {
        let chunks: &[&[u8]] = &[b"chunk-one-", b"chunk-two-", b"chunk-three"];
        let expected: Vec<u8> = chunks.iter().flat_map(|c| c.iter().copied()).collect();

        let mut blob = {
            let mut writer = BlobWriter::new(db).expect("BlobWriter::new");
            for chunk in chunks {
                writer.write_all(chunk).expect("write chunk");
            }
            Blob::new_from_stream(writer, "text/plain")
        };

        assert_eq!(blob.length(), expected.len() as u64);

        // Persist the blob by attaching it to a saved document.
        let mut doc = Document::new_with_id("multi_chunk_doc");
        doc.mutable_properties().at("data").put_blob(&mut blob);
        default_collection(db)
            .save_document_with_concurency_control(&mut doc, ConcurrencyControl::FailOnConflict)
            .expect("save");

        let reloaded = default_collection(db)
            .get_document("multi_chunk_doc")
            .expect("get_document");
        let stored = reloaded
            .properties()
            .get("data")
            .as_blob()
            .expect("blob property");

        assert_eq!(stored.load_content().expect("load_content"), expected);
    });
}

// ── BlobReader / open_content ─────────────────────────────────────────────────

// open_content() returns a streaming reader whose Read impl produces the same
// bytes as load_content().
#[test]
fn blob_reader_streaming_read() {
    utils::with_db(|db| {
        let data = b"stream this content back out";

        let mut blob = Blob::new_from_data(data, "text/plain");
        let mut doc = Document::new_with_id("reader_doc");
        doc.mutable_properties().at("b").put_blob(&mut blob);
        default_collection(db)
            .save_document_with_concurency_control(&mut doc, ConcurrencyControl::FailOnConflict)
            .expect("save");

        let doc = default_collection(db)
            .get_document("reader_doc")
            .expect("get_document");
        let stored = doc.properties().get("b").as_blob().expect("blob");

        let mut reader = stored.open_content().expect("open_content");
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).expect("read_to_end");

        assert_eq!(buf, data);
    });
}

// Reading into a small buffer exercises the chunked Read path.
#[test]
fn blob_reader_small_buffer() {
    utils::with_db(|db| {
        let data = b"abcdefghijklmnopqrstuvwxyz";

        let mut blob = Blob::new_from_data(data, "text/plain");
        let mut doc = Document::new_with_id("small_buf_doc");
        doc.mutable_properties().at("b").put_blob(&mut blob);
        default_collection(db)
            .save_document_with_concurency_control(&mut doc, ConcurrencyControl::FailOnConflict)
            .expect("save");

        let doc = default_collection(db)
            .get_document("small_buf_doc")
            .expect("get_document");
        let stored = doc.properties().get("b").as_blob().expect("blob");

        let mut reader = stored.open_content().expect("open_content");
        let mut result = Vec::new();
        let mut chunk = [0u8; 4];
        loop {
            let n = reader.read(&mut chunk).expect("read");
            if n == 0 {
                break;
            }
            result.extend_from_slice(&chunk[..n]);
        }

        assert_eq!(result, data);
    });
}
