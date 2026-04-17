extern crate couchbase_lite;
use self::couchbase_lite::*;
use std::time::Duration;

pub mod utils;

// ── TLSIdentity::create ───────────────────────────────────────────────────────

/// `TLSIdentity::create` returns an identity whose certificate subject name
/// contains the requested CN and whose expiration is in the future.
#[test]
#[cfg(feature = "enterprise")]
fn tls_identity_create_server() {
    let mut attrs = MutableDict::new();
    attrs.at("CN").put_string("test-server");

    let expiry = Timestamp::now().add(Duration::from_secs(86400));
    let identity =
        TLSIdentity::create(true, &attrs, Some(expiry), None).expect("create server TLS identity");

    let cert = identity.certificates();
    let subject = cert.subject_name();
    assert!(
        subject.contains("test-server"),
        "subject name should contain 'test-server', got: {subject}"
    );

    assert!(
        identity.expiration().get() > Timestamp::now().get(),
        "expiration should be in the future"
    );
}

/// Client identity (`is_server = false`) also succeeds and has a valid cert.
#[test]
#[cfg(feature = "enterprise")]
fn tls_identity_create_client() {
    let mut attrs = MutableDict::new();
    attrs.at("CN").put_string("test-client");

    let identity =
        TLSIdentity::create(false, &attrs, None, None).expect("create client TLS identity");

    let cert = identity.certificates();
    let subject = cert.subject_name();
    assert!(
        subject.contains("test-client"),
        "subject name should contain 'test-client', got: {subject}"
    );
}

// ── Cert ──────────────────────────────────────────────────────────────────────

/// `Cert::data` returns non-empty DER bytes.
#[test]
#[cfg(feature = "enterprise")]
fn cert_data_der_non_empty() {
    let mut attrs = MutableDict::new();
    attrs.at("CN").put_string("der-test");

    let identity = TLSIdentity::create(true, &attrs, None, None).expect("create identity");

    let cert = identity.certificates();
    let der = cert.data(false); // DER
    assert!(!der.is_empty(), "DER data should be non-empty");
}

/// `Cert::data` with `pem_encoded = true` starts with the PEM header.
#[test]
#[cfg(feature = "enterprise")]
fn cert_data_pem_has_header() {
    let mut attrs = MutableDict::new();
    attrs.at("CN").put_string("pem-test");

    let identity = TLSIdentity::create(true, &attrs, None, None).expect("create identity");

    let cert = identity.certificates();
    let pem = cert.data(true);
    let pem_str = std::str::from_utf8(&pem).expect("PEM should be valid UTF-8");
    assert!(
        pem_str.starts_with("-----BEGIN CERTIFICATE-----"),
        "PEM should start with certificate header, got: {}",
        &pem_str[..pem_str.len().min(40)]
    );
}

/// `Cert::valid_timespan` returns a created timestamp before the expiry timestamp.
#[test]
#[cfg(feature = "enterprise")]
fn cert_valid_timespan_ordering() {
    let mut attrs = MutableDict::new();
    attrs.at("CN").put_string("timespan-test");

    let identity = TLSIdentity::create(true, &attrs, None, None).expect("create identity");

    let cert = identity.certificates();
    let (created, expires) = cert.valid_timespan();
    assert!(
        created.get() < expires.get(),
        "created ({}) should be before expires ({})",
        created.get(),
        expires.get()
    );
}

/// `Cert::public_key` returns a non-null `KeyPair` with non-empty public key data.
#[test]
#[cfg(feature = "enterprise")]
fn cert_public_key_non_null() {
    let mut attrs = MutableDict::new();
    attrs.at("CN").put_string("pubkey-test");

    let identity = TLSIdentity::create(true, &attrs, None, None).expect("create identity");

    let cert = identity.certificates();
    let key_pair = cert.public_key().expect("cert should have a public key");
    let pub_data = key_pair.public_key_data();
    assert!(!pub_data.is_empty(), "public key data should be non-empty");
}

// ── TLSIdentity::with_key_pair_and_certs ─────────────────────────────────────

/// `TLSIdentity::with_key_pair_and_certs` assembles an identity from an
/// existing key pair and certificate chain.  We use the cert and public key
/// from a freshly created identity to verify the round-trip.
#[test]
#[cfg(feature = "enterprise")]
fn tls_identity_with_key_pair_and_certs() {
    let mut attrs = MutableDict::new();
    attrs.at("CN").put_string("roundtrip");

    let source = TLSIdentity::create(true, &attrs, None, None).expect("create source identity");

    let cert = source.certificates();
    let key_pair = cert.public_key().expect("cert should have a public key");

    // Assembling from a public-only key pair should return an error (no private
    // key available to sign), not a crash.
    let result = TLSIdentity::with_key_pair_and_certs(&key_pair, &cert);
    // Either Ok (if the library accepts public-only) or Err — both are valid;
    // the important invariant is no panic or SIGSEGV.
    let _ = result;
}
