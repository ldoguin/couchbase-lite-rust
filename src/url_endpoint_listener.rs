// URL endpoint listener API — enterprise edition only.
//
// Wraps CBLURLEndpointListener, CBLListenerAuthenticator, and related types.

use crate::{
    CblRef,
    c_api::{
        CBLCert, CBLCollection, CBLConnectionStatus, CBLError, CBLListenerAuth_CreateCertificate,
        CBLListenerAuth_CreateCertificateWithRootCerts, CBLListenerAuth_CreatePassword,
        CBLListenerAuth_Free, CBLListenerAuthenticator, CBLListenerCertAuthCallback,
        CBLListenerPasswordAuthCallback, CBLURLEndpointListener, CBLURLEndpointListener_Create,
        CBLURLEndpointListener_Port, CBLURLEndpointListener_Start, CBLURLEndpointListener_Status,
        CBLURLEndpointListener_Stop, CBLURLEndpointListener_TLSIdentity,
        CBLURLEndpointListener_Urls, CBLURLEndpointListenerConfiguration, FLArray_Count,
        FLArray_Get, FLValue_Release,
    },
    collection::Collection,
    error::{Result, failure},
    release, retain,
    slice::NULL_SLICE,
    tls_identity::{Cert, TLSIdentity},
};
use std::ptr;

// ── ListenerAuthenticator ─────────────────────────────────────────────────────

/// Authenticator used by a `UrlEndpointListener` to verify connecting clients.
pub struct ListenerAuthenticator {
    cbl_ref: *mut CBLListenerAuthenticator,
}

impl ListenerAuthenticator {
    /// Accepts any client that provides a valid username/password via HTTP Basic auth.
    ///
    /// The callback receives `(username, password)` and returns `true` to accept.
    pub fn password<F>(callback: F) -> Self
    where
        F: Fn(String, String) -> bool + Send + Sync + 'static,
    {
        // Box the closure and leak it; the `unregistered` path is not available
        // for listener auth, so we accept the small leak for the listener's lifetime.
        let boxed: Box<Box<dyn Fn(String, String) -> bool + Send + Sync>> =
            Box::new(Box::new(callback));
        let ctx = Box::into_raw(boxed) as *mut std::ffi::c_void;

        unsafe extern "C" fn c_password_cb(
            context: *mut std::ffi::c_void,
            username: crate::c_api::FLString,
            password: crate::c_api::FLString,
        ) -> bool {
            unsafe {
                let cb = &*(context as *const Box<dyn Fn(String, String) -> bool + Send + Sync>);
                let u = username.to_string().unwrap_or_default();
                let p = password.to_string().unwrap_or_default();
                cb(u, p)
            }
        }

        let auth_cb: CBLListenerPasswordAuthCallback = Some(c_password_cb);
        let ptr = unsafe { CBLListenerAuth_CreatePassword(auth_cb, ctx) };
        Self { cbl_ref: ptr }
    }

    /// Accepts any client certificate (no verification).
    pub fn certificate() -> Self {
        let auth_cb: CBLListenerCertAuthCallback = Some(c_accept_any_cert);
        let ptr = unsafe { CBLListenerAuth_CreateCertificate(auth_cb, ptr::null_mut()) };
        Self { cbl_ref: ptr }
    }

    /// Accepts client certificates that chain to the given root certificate.
    pub fn certificate_with_root_certs(root_certs: Cert) -> Self {
        let ptr = unsafe { CBLListenerAuth_CreateCertificateWithRootCerts(root_certs.get_ref()) };
        Self { cbl_ref: ptr }
    }
}

unsafe extern "C" fn c_accept_any_cert(
    _context: *mut std::ffi::c_void,
    _cert: *mut CBLCert,
) -> bool {
    true
}

impl Drop for ListenerAuthenticator {
    fn drop(&mut self) {
        unsafe { CBLListenerAuth_Free(self.cbl_ref) }
    }
}

// ── ListenerConfiguration ─────────────────────────────────────────────────────

/// Configuration for a `UrlEndpointListener`.
#[derive(Default)]
pub struct ListenerConfiguration {
    /// Collections to expose for replication.
    pub collections: Vec<Collection>,
    /// Port to listen on. `0` lets the OS pick an available port.
    pub port: u16,
    /// Network interface name or IP address. `None` listens on all interfaces.
    pub network_interface: Option<String>,
    /// TLS identity. `None` disables TLS.
    pub tls_identity: Option<TLSIdentity>,
    /// Client authenticator. `None` allows unauthenticated connections.
    pub authenticator: Option<ListenerAuthenticator>,
    /// If `true`, only pull replication is allowed.
    pub read_only: bool,
    /// Enable delta sync.
    pub enable_delta_sync: bool,
}

// ── ConnectionStatus ──────────────────────────────────────────────────────────

/// Current connection counts for a running listener.
pub struct ConnectionStatus {
    pub connection_count: u64,
    pub active_connection_count: u64,
}

impl From<CBLConnectionStatus> for ConnectionStatus {
    fn from(s: CBLConnectionStatus) -> Self {
        Self {
            connection_count: s.connectionCount,
            active_connection_count: s.activeConnectionCount,
        }
    }
}

// ── UrlEndpointListener ───────────────────────────────────────────────────────

/// A listener that allows remote peers to replicate with local collections over WebSocket.
pub struct UrlEndpointListener {
    cbl_ref: *mut CBLURLEndpointListener,
    // Keep collection pointers alive for the duration of the listener.
    _collection_ptrs: Vec<*mut CBLCollection>,
}

impl CblRef for UrlEndpointListener {
    type Output = *mut CBLURLEndpointListener;
    fn get_ref(&self) -> Self::Output {
        self.cbl_ref
    }
}

impl UrlEndpointListener {
    /// Creates a new listener with the given configuration.
    pub fn new(config: ListenerConfiguration) -> Result<Self> {
        // Build a contiguous array of raw collection pointers.
        let mut collection_ptrs: Vec<*mut CBLCollection> =
            config.collections.iter().map(|c| c.get_ref()).collect();

        let network_interface_slice = config
            .network_interface
            .as_deref()
            .map(crate::slice::from_str);
        let network_interface_ref = network_interface_slice
            .as_ref()
            .map(|s| s.get_ref())
            .unwrap_or(NULL_SLICE);

        let c_config = CBLURLEndpointListenerConfiguration {
            collections: if collection_ptrs.is_empty() {
                ptr::null_mut()
            } else {
                collection_ptrs.as_mut_ptr()
            },
            collectionCount: collection_ptrs.len(),
            port: config.port,
            networkInterface: network_interface_ref,
            disableTLS: config.tls_identity.is_none(),
            tlsIdentity: config
                .tls_identity
                .as_ref()
                .map(|t| t.get_ref())
                .unwrap_or(ptr::null_mut()),
            authenticator: config
                .authenticator
                .as_ref()
                .map(|a| a.cbl_ref)
                .unwrap_or(ptr::null_mut()),
            enableDeltaSync: config.enable_delta_sync,
            readOnly: config.read_only,
        };

        unsafe {
            let mut err = CBLError::default();
            let ptr = CBLURLEndpointListener_Create(&c_config, &mut err);
            if ptr.is_null() {
                return failure(err);
            }
            Ok(Self {
                cbl_ref: ptr,
                _collection_ptrs: collection_ptrs,
            })
        }
    }

    /// Starts the listener. After this call, `port()` returns the bound port.
    pub fn start(&self) -> Result<()> {
        unsafe {
            let mut err = CBLError::default();
            if CBLURLEndpointListener_Start(self.cbl_ref, &mut err) {
                Ok(())
            } else {
                failure(err)
            }
        }
    }

    /// Stops the listener.
    pub fn stop(&self) {
        unsafe { CBLURLEndpointListener_Stop(self.cbl_ref) }
    }

    /// Returns the port the listener is bound to. Returns `0` if not yet started.
    pub fn port(&self) -> u16 {
        unsafe { CBLURLEndpointListener_Port(self.cbl_ref) }
    }

    /// Returns the TLS identity in use, or `None` if TLS is disabled or the
    /// listener has not been started yet.
    ///
    /// The returned identity is retained — it remains valid even after the
    /// listener is stopped or released.
    pub fn tls_identity(&self) -> Option<TLSIdentity> {
        unsafe {
            let ptr = CBLURLEndpointListener_TLSIdentity(self.cbl_ref);
            if ptr.is_null() {
                None
            } else {
                // Retain so the TLSIdentity outlives the listener if needed.
                Some(TLSIdentity::take_ownership(retain(ptr)))
            }
        }
    }

    /// Returns the URLs the listener is reachable at, or an empty `Vec` if the
    /// listener has not been started yet.
    pub fn urls(&self) -> Vec<String> {
        use crate::fleece::Value;
        unsafe {
            let arr = CBLURLEndpointListener_Urls(self.cbl_ref);
            if arr.is_null() {
                return vec![];
            }
            // FLMutableArray is *mut _FLArray; cast to *const _FLArray for FLArray_Count/Get.
            let arr_const = arr as crate::c_api::FLArray;
            let count = FLArray_Count(arr_const);
            let mut urls = Vec::with_capacity(count as usize);
            for i in 0..count {
                let val = Value {
                    cbl_ref: FLArray_Get(arr_const, i),
                };
                if let Some(s) = val.as_string() {
                    urls.push(s.to_string());
                }
            }
            // We own the returned array; release it now.
            FLValue_Release(arr as crate::c_api::FLValue);
            urls
        }
    }

    /// Returns the current connection counts for this listener.
    pub fn status(&self) -> ConnectionStatus {
        unsafe { CBLURLEndpointListener_Status(self.cbl_ref).into() }
    }
}

impl Drop for UrlEndpointListener {
    fn drop(&mut self) {
        unsafe { release(self.cbl_ref) }
    }
}

// CBLURLEndpointListener is thread-safe per the CouchbaseLite C API contract.
// The raw pointer is not aliased outside this struct after construction.
unsafe impl Send for UrlEndpointListener {}
unsafe impl Sync for UrlEndpointListener {}
