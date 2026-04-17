extern crate couchbase_lite;
use self::couchbase_lite::*;
use std::time::Duration;

pub mod utils;

// ── basic start / stop ────────────────────────────────────────────────────────

/// A listener with `port = 0` binds to an OS-assigned port and reports it
/// via `port()` after `start()`.
#[test]
#[cfg(feature = "enterprise")]
fn listener_starts_and_reports_port() {
    utils::with_db(|db| {
        let coll = db.default_collection_or_error().unwrap();

        let config = ListenerConfiguration {
            collections: vec![coll],
            port: 0,
            tls_identity: None,
            authenticator: None,
            ..Default::default()
        };

        let listener = UrlEndpointListener::new(config).expect("create listener");
        listener.start().expect("start listener");

        let port = listener.port();
        assert!(port > 0, "expected non-zero port after start, got {port}");

        listener.stop();
    });
}

/// `stop()` is idempotent — calling it twice must not panic or crash.
#[test]
#[cfg(feature = "enterprise")]
fn listener_stop_is_idempotent() {
    utils::with_db(|db| {
        let coll = db.default_collection_or_error().unwrap();

        let config = ListenerConfiguration {
            collections: vec![coll],
            port: 0,
            tls_identity: None,
            authenticator: None,
            ..Default::default()
        };

        let listener = UrlEndpointListener::new(config).expect("create listener");
        listener.start().expect("start listener");
        listener.stop();
        listener.stop(); // second call must not crash
    });
}

// ── port() before start ───────────────────────────────────────────────────────

/// `port()` returns 0 before `start()` is called.
#[test]
#[cfg(feature = "enterprise")]
fn listener_port_zero_before_start() {
    utils::with_db(|db| {
        let coll = db.default_collection_or_error().unwrap();

        let config = ListenerConfiguration {
            collections: vec![coll],
            port: 0,
            tls_identity: None,
            authenticator: None,
            ..Default::default()
        };

        let listener = UrlEndpointListener::new(config).expect("create listener");
        assert_eq!(listener.port(), 0, "port should be 0 before start");
    });
}

// ── fixed port ────────────────────────────────────────────────────────────────

/// When a specific port is requested, `port()` returns that same port.
#[test]
#[cfg(feature = "enterprise")]
fn listener_fixed_port() {
    utils::with_db(|db| {
        let coll = db.default_collection_or_error().unwrap();

        // Pick a port unlikely to be in use; if it is, the test is skipped.
        let requested: u16 = 59840;

        let config = ListenerConfiguration {
            collections: vec![coll],
            port: requested,
            tls_identity: None,
            authenticator: None,
            ..Default::default()
        };

        let listener = match UrlEndpointListener::new(config) {
            Ok(l) => l,
            Err(_) => return, // port already in use — skip
        };

        match listener.start() {
            Ok(()) => {
                assert_eq!(
                    listener.port(),
                    requested,
                    "expected port {requested}, got {}",
                    listener.port()
                );
                listener.stop();
            }
            Err(_) => {} // port in use — skip
        }
    });
}

// ── TLS listener ──────────────────────────────────────────────────────────────

/// A listener with a TLS identity starts successfully.
#[test]
#[cfg(feature = "enterprise")]
fn listener_with_tls_identity() {
    utils::with_db(|db| {
        let coll = db.default_collection_or_error().unwrap();

        let mut attrs = MutableDict::new();
        attrs.at("CN").put_string("listener-tls-test");

        let expiry = Timestamp::now().add(Duration::from_secs(86400));
        let identity =
            TLSIdentity::create(true, &attrs, Some(expiry), None).expect("create TLS identity");

        let config = ListenerConfiguration {
            collections: vec![coll],
            port: 0,
            tls_identity: Some(identity),
            authenticator: None,
            ..Default::default()
        };

        let listener = UrlEndpointListener::new(config).expect("create TLS listener");
        listener.start().expect("start TLS listener");

        assert!(
            listener.port() > 0,
            "TLS listener should report a non-zero port"
        );

        listener.stop();
    });
}

// ── password authenticator ────────────────────────────────────────────────────

/// `urls()` returns at least one URL after the listener starts.
#[test]
#[cfg(feature = "enterprise")]
fn listener_urls_non_empty_after_start() {
    utils::with_db(|db| {
        let coll = db.default_collection_or_error().unwrap();

        let config = ListenerConfiguration {
            collections: vec![coll],
            port: 0,
            tls_identity: None,
            authenticator: None,
            ..Default::default()
        };

        let listener = UrlEndpointListener::new(config).expect("create listener");
        listener.start().expect("start listener");

        let urls = listener.urls();
        assert!(!urls.is_empty(), "expected at least one URL after start");
        for url in &urls {
            assert!(
                url.starts_with("ws://") || url.starts_with("wss://"),
                "URL should start with ws:// or wss://, got: {url}"
            );
        }

        listener.stop();
    });
}

/// `urls()` returns an empty Vec before the listener is started.
#[test]
#[cfg(feature = "enterprise")]
fn listener_urls_empty_before_start() {
    utils::with_db(|db| {
        let coll = db.default_collection_or_error().unwrap();

        let config = ListenerConfiguration {
            collections: vec![coll],
            port: 0,
            tls_identity: None,
            authenticator: None,
            ..Default::default()
        };

        let listener = UrlEndpointListener::new(config).expect("create listener");
        assert!(
            listener.urls().is_empty(),
            "expected empty URLs before start"
        );
    });
}

/// `status()` reports zero connections on a freshly started listener.
#[test]
#[cfg(feature = "enterprise")]
fn listener_status_zero_connections() {
    utils::with_db(|db| {
        let coll = db.default_collection_or_error().unwrap();

        let config = ListenerConfiguration {
            collections: vec![coll],
            port: 0,
            tls_identity: None,
            authenticator: None,
            ..Default::default()
        };

        let listener = UrlEndpointListener::new(config).expect("create listener");
        listener.start().expect("start listener");

        let status = listener.status();
        assert_eq!(
            status.connection_count, 0,
            "expected 0 connections on idle listener"
        );
        assert_eq!(
            status.active_connection_count, 0,
            "expected 0 active connections on idle listener"
        );

        listener.stop();
    });
}

/// `tls_identity()` returns `None` when TLS is disabled.
#[test]
#[cfg(feature = "enterprise")]
fn listener_tls_identity_none_when_disabled() {
    utils::with_db(|db| {
        let coll = db.default_collection_or_error().unwrap();

        let config = ListenerConfiguration {
            collections: vec![coll],
            port: 0,
            tls_identity: None, // TLS disabled
            authenticator: None,
            ..Default::default()
        };

        let listener = UrlEndpointListener::new(config).expect("create listener");
        listener.start().expect("start listener");

        assert!(
            listener.tls_identity().is_none(),
            "expected None tls_identity when TLS is disabled"
        );

        listener.stop();
    });
}

/// `tls_identity()` returns `Some` when a TLS identity is configured.
#[test]
#[cfg(feature = "enterprise")]
fn listener_tls_identity_some_when_enabled() {
    use std::time::Duration;

    utils::with_db(|db| {
        let coll = db.default_collection_or_error().unwrap();

        let mut attrs = MutableDict::new();
        attrs.at("CN").put_string("tls-identity-test");

        let expiry = Timestamp::now().add(Duration::from_secs(86400));
        let identity =
            TLSIdentity::create(true, &attrs, Some(expiry), None).expect("create TLS identity");

        let config = ListenerConfiguration {
            collections: vec![coll],
            port: 0,
            tls_identity: Some(identity),
            authenticator: None,
            ..Default::default()
        };

        let listener = UrlEndpointListener::new(config).expect("create TLS listener");
        listener.start().expect("start TLS listener");

        let returned = listener.tls_identity();
        assert!(
            returned.is_some(),
            "expected Some tls_identity when TLS is enabled"
        );

        listener.stop();
    });
}

/// A listener with a password authenticator starts without error.
#[test]
#[cfg(feature = "enterprise")]
fn listener_with_password_authenticator() {
    utils::with_db(|db| {
        let coll = db.default_collection_or_error().unwrap();

        let auth =
            ListenerAuthenticator::password(|user, pass| user == "alice" && pass == "secret");

        let config = ListenerConfiguration {
            collections: vec![coll],
            port: 0,
            tls_identity: None,
            authenticator: Some(auth),
            ..Default::default()
        };

        let listener = UrlEndpointListener::new(config).expect("create listener with auth");
        listener.start().expect("start listener with auth");
        assert!(listener.port() > 0);
        listener.stop();
    });
}
