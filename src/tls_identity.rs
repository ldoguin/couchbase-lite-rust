// TLS identity API — enterprise edition only.
//
// Wraps CBLCert, CBLKeyPair, and CBLTLSIdentity.

use crate::{
    CblRef, MutableDict, Timestamp,
    c_api::{
        CBLCert, CBLCert_CertNextInChain, CBLCert_Data, CBLCert_PublicKey, CBLCert_SubjectName,
        CBLCert_SubjectNameComponent, CBLCert_ValidTimespan, CBLCert_CreateWithData, CBLError,
        CBLKeyPair, CBLKeyPair_CreateWithPrivateKeyData, CBLKeyPair_PrivateKeyData,
        CBLKeyPair_PublicKeyData, CBLKeyPair_PublicKeyDigest, CBLTLSIdentity,
        CBLTLSIdentity_Certificates, CBLTLSIdentity_CreateIdentity,
        CBLTLSIdentity_CreateIdentityWithKeyPair, CBLTLSIdentity_Expiration,
        CBLTLSIdentity_IdentityWithKeyPairAndCerts, kCBLKeyUsagesClientAuth,
        kCBLKeyUsagesServerAuth,
    },
    error::{Result, failure},
    release, retain,
    slice::from_str,
};

// ── Cert ─────────────────────────────────────────────────────────────────────

/// An X.509 certificate (or chain of certificates).
pub struct Cert {
    cbl_ref: *mut CBLCert,
}

impl CblRef for Cert {
    type Output = *mut CBLCert;
    fn get_ref(&self) -> Self::Output {
        self.cbl_ref
    }
}

impl Cert {
    pub(crate) const fn take_ownership(cbl_ref: *mut CBLCert) -> Self {
        Self { cbl_ref }
    }

    pub(crate) fn reference(cbl_ref: *mut CBLCert) -> Self {
        Self {
            cbl_ref: unsafe { retain(cbl_ref) },
        }
    }

    /// Creates a `Cert` from X.509 certificate data in DER or PEM format.
    pub fn from_data(der: &[u8]) -> Result<Self> {
        unsafe {
            let mut err = CBLError::default();
            let slice = crate::slice::from_bytes(der);
            let ptr = CBLCert_CreateWithData(slice.get_ref(), &mut err);
            if ptr.is_null() {
                return failure(err);
            }
            Ok(Self::take_ownership(ptr))
        }
    }

    /// Returns the next certificate in the chain, if any.
    pub fn next_in_chain(&self) -> Option<Cert> {
        unsafe {
            let ptr = CBLCert_CertNextInChain(self.cbl_ref);
            if ptr.is_null() {
                None
            } else {
                // CBLCert_CertNextInChain returns a new reference — take ownership.
                Some(Self::take_ownership(ptr))
            }
        }
    }

    /// Returns the certificate data in DER (`pem_encoded = false`) or PEM format.
    pub fn data(&self, pem_encoded: bool) -> Vec<u8> {
        unsafe {
            let result = CBLCert_Data(self.cbl_ref, pem_encoded);
            result.to_vec().unwrap_or_default()
        }
    }

    /// Returns the certificate's Subject Name string.
    pub fn subject_name(&self) -> String {
        unsafe {
            let result = CBLCert_SubjectName(self.cbl_ref);
            result.to_string().unwrap_or_default()
        }
    }

    /// Returns a single component of the Subject Name identified by its OID key.
    pub fn subject_name_component(&self, oid: &str) -> Option<String> {
        unsafe {
            let result = CBLCert_SubjectNameComponent(self.cbl_ref, from_str(oid).get_ref());
            result.to_string()
        }
    }

    /// Returns the (created, expires) validity timespan as `Timestamp` values.
    pub fn valid_timespan(&self) -> (Timestamp, Timestamp) {
        unsafe {
            let mut created: i64 = 0;
            let mut expires: i64 = 0;
            CBLCert_ValidTimespan(self.cbl_ref, &mut created, &mut expires);
            (Timestamp::new(created), Timestamp::new(expires))
        }
    }

    /// Returns the public key embedded in this certificate, if available.
    pub fn public_key(&self) -> Option<KeyPair> {
        unsafe {
            let ptr = CBLCert_PublicKey(self.cbl_ref);
            if ptr.is_null() {
                None
            } else {
                Some(KeyPair::take_ownership(ptr))
            }
        }
    }
}

impl Drop for Cert {
    fn drop(&mut self) {
        unsafe { release(self.cbl_ref) }
    }
}

// ── KeyPair ───────────────────────────────────────────────────────────────────

/// An RSA key pair (public + optional private key).
pub struct KeyPair {
    cbl_ref: *mut CBLKeyPair,
}

impl CblRef for KeyPair {
    type Output = *mut CBLKeyPair;
    fn get_ref(&self) -> Self::Output {
        self.cbl_ref
    }
}

impl KeyPair {
    pub(crate) const fn take_ownership(cbl_ref: *mut CBLKeyPair) -> Self {
        Self { cbl_ref }
    }

    /// Creates a `KeyPair` from private key data in PEM or DER format.
    pub fn from_private_key_data(private_key: &[u8], password: Option<&str>) -> Result<Self> {
        unsafe {
            let mut err = CBLError::default();
            let key_slice = crate::slice::from_bytes(private_key);
            let pwd_slice = password.map(from_str);
            let pwd_ref = pwd_slice
                .as_ref()
                .map(|s| s.get_ref())
                .unwrap_or(crate::slice::NULL_SLICE);
            let ptr = CBLKeyPair_CreateWithPrivateKeyData(key_slice.get_ref(), pwd_ref, &mut err);
            if ptr.is_null() {
                return failure(err);
            }
            Ok(Self::take_ownership(ptr))
        }
    }

    /// Returns the public key data as a DER-encoded SubjectPublicKeyInfo structure.
    pub fn public_key_data(&self) -> Vec<u8> {
        unsafe {
            let result = CBLKeyPair_PublicKeyData(self.cbl_ref);
            result.to_vec().unwrap_or_default()
        }
    }

    /// Returns the private key data, if available.
    pub fn private_key_data(&self) -> Option<Vec<u8>> {
        unsafe {
            let result = CBLKeyPair_PrivateKeyData(self.cbl_ref);
            result.to_vec()
        }
    }

    /// Returns the public key digest.
    pub fn public_key_digest(&self) -> Vec<u8> {
        unsafe {
            let result = CBLKeyPair_PublicKeyDigest(self.cbl_ref);
            result.to_vec().unwrap_or_default()
        }
    }
}

impl Drop for KeyPair {
    fn drop(&mut self) {
        unsafe { release(self.cbl_ref) }
    }
}

// ── TLSIdentity ───────────────────────────────────────────────────────────────

/// A TLS identity consisting of a key pair and certificate chain.
pub struct TLSIdentity {
    cbl_ref: *mut CBLTLSIdentity,
}

impl CblRef for TLSIdentity {
    type Output = *mut CBLTLSIdentity;
    fn get_ref(&self) -> Self::Output {
        self.cbl_ref
    }
}

impl TLSIdentity {
    pub(crate) const fn take_ownership(cbl_ref: *mut CBLTLSIdentity) -> Self {
        Self { cbl_ref }
    }

    /// Creates a self-signed TLS identity.
    ///
    /// `is_server` selects server-auth vs client-auth key usage.
    /// `attributes` must contain at least `CN` (Common Name).
    /// `expiration` sets the certificate validity end; `None` uses a 1-year default.
    /// `label` is not supported on Linux — pass `None`.
    pub fn create(
        is_server: bool,
        attributes: &MutableDict,
        expiration: Option<Timestamp>,
        label: Option<&str>,
    ) -> Result<Self> {
        unsafe {
            let key_usages: u16 = if is_server {
                kCBLKeyUsagesServerAuth as u16
            } else {
                kCBLKeyUsagesClientAuth as u16
            };

            // Default validity: 365 days in milliseconds.
            let validity_ms: i64 = expiration
                .map(|e| e.get() - Timestamp::now().get())
                .unwrap_or(365 * 24 * 3600 * 1000);

            let label_slice = label.map(from_str);
            let label_ref = label_slice
                .as_ref()
                .map(|s| s.get_ref())
                .unwrap_or(crate::slice::NULL_SLICE);

            let mut err = CBLError::default();
            let ptr = CBLTLSIdentity_CreateIdentity(
                key_usages,
                attributes.as_dict().get_ref(),
                validity_ms,
                label_ref,
                &mut err,
            );
            if ptr.is_null() {
                return failure(err);
            }
            Ok(Self::take_ownership(ptr))
        }
    }

    /// Creates a self-signed TLS identity using an existing key pair.
    pub fn create_with_key_pair(
        key_pair: &KeyPair,
        attributes: &MutableDict,
        expiration: Option<Timestamp>,
    ) -> Result<Self> {
        unsafe {
            let key_usages: u16 = kCBLKeyUsagesServerAuth as u16;
            let validity_ms: i64 = expiration
                .map(|e| e.get() - Timestamp::now().get())
                .unwrap_or(365 * 24 * 3600 * 1000);

            let mut err = CBLError::default();
            let ptr = CBLTLSIdentity_CreateIdentityWithKeyPair(
                key_usages,
                key_pair.cbl_ref,
                attributes.as_dict().get_ref(),
                validity_ms,
                &mut err,
            );
            if ptr.is_null() {
                return failure(err);
            }
            Ok(Self::take_ownership(ptr))
        }
    }

    /// Returns a TLS identity from an existing key pair and certificate chain.
    pub fn with_key_pair_and_certs(key_pair: &KeyPair, certs: &Cert) -> Result<Self> {
        unsafe {
            let mut err = CBLError::default();
            let ptr = CBLTLSIdentity_IdentityWithKeyPairAndCerts(
                key_pair.cbl_ref,
                certs.cbl_ref,
                &mut err,
            );
            if ptr.is_null() {
                return failure(err);
            }
            Ok(Self::take_ownership(ptr))
        }
    }

    /// Returns the first certificate in the identity's chain.
    pub fn certificates(&self) -> Cert {
        unsafe {
            let ptr = CBLTLSIdentity_Certificates(self.cbl_ref);
            // The returned pointer is not retained by the C API — retain it.
            Cert::reference(ptr)
        }
    }

    /// Returns the expiration timestamp of the first certificate.
    pub fn expiration(&self) -> Timestamp {
        unsafe { Timestamp::new(CBLTLSIdentity_Expiration(self.cbl_ref)) }
    }
}

impl Drop for TLSIdentity {
    fn drop(&mut self) {
        unsafe { release(self.cbl_ref) }
    }
}
