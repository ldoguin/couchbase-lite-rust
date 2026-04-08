// Vector search API — enterprise edition only.
//
// Wraps CBLVectorEncoding, CBLVectorIndexConfiguration, CBLIndexUpdater,
// and the CBL_EnableVectorSearch free function.

use crate::{
    CblRef,
    c_api::{
        CBLError, CBLIndexUpdater, CBLIndexUpdater_Count, CBLIndexUpdater_Finish,
        CBLIndexUpdater_SetVector, CBLIndexUpdater_SkipVector, CBLIndexUpdater_Value,
        CBLQueryIndex_BeginUpdate, CBLVectorEncoding, CBLVectorEncoding_CreateNone,
        CBLVectorEncoding_CreateProductQuantizer, CBLVectorEncoding_CreateScalarQuantizer,
        CBLVectorEncoding_Free, CBLVectorIndexConfiguration, CBL_EnableVectorSearch,
        kCBLDistanceMetricCosine, kCBLDistanceMetricDot,
        kCBLDistanceMetricEuclidean, kCBLDistanceMetricEuclideanSquared,
        kCBLSQ4, kCBLSQ6, kCBLSQ8, CBLCollection_CreateVectorIndex,
    },
    collection::Collection,
    error::{Result, failure},
    fleece::Value,
    index::QueryIndex,
    release,
    slice::from_str,
};
use std::ptr;

// ── enable_vector_search ──────────────────────────────────────────────────────

/// Loads the vector search extension from the given directory path.
///
/// Must be called before opening any database that uses vector indexes.
pub fn enable_vector_search(extension_path: &str) -> Result<()> {
    unsafe {
        let mut err = CBLError::default();
        if CBL_EnableVectorSearch(from_str(extension_path).get_ref(), &mut err) {
            Ok(())
        } else {
            failure(err)
        }
    }
}

// ── VectorEncoding ────────────────────────────────────────────────────────────

/// Scalar quantizer type for `VectorEncoding::ScalarQuantizer`.
#[derive(Debug, Clone, Copy)]
pub enum ScalarQuantizerType {
    SQ4 = kCBLSQ4 as isize,
    SQ6 = kCBLSQ6 as isize,
    SQ8 = kCBLSQ8 as isize,
}

/// Encoding applied to vectors before storing them in the index.
pub enum VectorEncoding {
    /// No encoding — 4 bytes per dimension, lossless.
    None,
    /// Scalar quantizer encoding.
    ScalarQuantizer(ScalarQuantizerType),
    /// Product quantizer encoding.
    ProductQuantizer {
        /// Number of subquantizers (must be > 1 and a factor of dimensions).
        subquantizers: u32,
        /// Bits per subquantizer (4–12).
        bits: u32,
    },
}

impl VectorEncoding {
    /// Converts to a raw `CBLVectorEncoding` pointer. Caller must call
    /// `CBLVectorEncoding_Free` when done (after the index is created).
    pub(crate) unsafe fn to_raw(&self) -> *mut CBLVectorEncoding {
        unsafe {
            match self {
                VectorEncoding::None => CBLVectorEncoding_CreateNone(),
                VectorEncoding::ScalarQuantizer(t) => {
                    CBLVectorEncoding_CreateScalarQuantizer(*t as u32)
                }
                VectorEncoding::ProductQuantizer { subquantizers, bits } => {
                    CBLVectorEncoding_CreateProductQuantizer(*subquantizers, *bits)
                }
            }
        }
    }
}

// ── DistanceMetric ────────────────────────────────────────────────────────────

/// Distance metric used when comparing vectors.
#[derive(Debug, Clone, Copy)]
pub enum DistanceMetric {
    EuclideanSquared = kCBLDistanceMetricEuclideanSquared as isize,
    Cosine = kCBLDistanceMetricCosine as isize,
    Euclidean = kCBLDistanceMetricEuclidean as isize,
    Dot = kCBLDistanceMetricDot as isize,
}

// ── VectorIndexConfiguration ──────────────────────────────────────────────────

/// Configuration for a vector index.
pub struct VectorIndexConfiguration {
    /// Query language used in `expression` (as the raw C integer value).
    pub expression_language: u32,
    /// Expression that evaluates to a vector (array of f32) for each document.
    pub expression: String,
    /// Number of dimensions in each vector.
    pub dimensions: u32,
    /// Number of centroids (buckets). Recommended: sqrt(number of vectors).
    pub centroids: u32,
    /// Encoding applied to stored vectors.
    pub encoding: VectorEncoding,
    /// Distance metric.
    pub metric: DistanceMetric,
    /// Minimum vectors required before training. `0` = auto.
    pub min_training_size: u32,
    /// Maximum vectors used for training. `0` = auto.
    pub max_training_size: u32,
    /// Number of centroids scanned per query. `0` = auto.
    pub num_probes: u32,
    /// If `true`, the index is lazy and must be updated manually via `IndexUpdater`.
    pub lazy: bool,
}

impl VectorIndexConfiguration {
    /// Creates a minimal non-lazy vector index configuration using N1QL expression language.
    pub fn new(expression: &str, dimensions: u32, centroids: u32) -> Self {
        // N1QL = 1 (kCBLN1QLLanguage)
        Self {
            expression_language: 1,
            expression: expression.to_string(),
            dimensions,
            centroids,
            encoding: VectorEncoding::None,
            metric: DistanceMetric::EuclideanSquared,
            min_training_size: 0,
            max_training_size: 0,
            num_probes: 0,
            lazy: false,
        }
    }
}

// ── Collection::create_vector_index ──────────────────────────────────────────

impl Collection {
    /// Creates a vector index in this collection.
    ///
    /// If an identical index already exists, this is a no-op.
    /// If a different index with the same name exists, it is replaced.
    pub fn create_vector_index(
        &self,
        name: &str,
        config: &VectorIndexConfiguration,
    ) -> Result<()> {
        unsafe {
            let encoding_ptr = config.encoding.to_raw();
            let expr_slice = from_str(&config.expression);
            let c_config = CBLVectorIndexConfiguration {
                expressionLanguage: config.expression_language,
                expression: expr_slice.get_ref(),
                dimensions: config.dimensions,
                centroids: config.centroids,
                isLazy: config.lazy,
                encoding: encoding_ptr,
                metric: config.metric as u32,
                minTrainingSize: config.min_training_size,
                maxTrainingSize: config.max_training_size,
                numProbes: config.num_probes,
            };

            let mut err = CBLError::default();
            let ok = CBLCollection_CreateVectorIndex(
                self.get_ref(),
                from_str(name).get_ref(),
                c_config,
                &mut err,
            );

            // Free the encoding object now that the index has been created.
            if !encoding_ptr.is_null() {
                CBLVectorEncoding_Free(encoding_ptr);
            }

            if ok {
                Ok(())
            } else {
                failure(err)
            }
        }
    }
}

// ── IndexUpdater ──────────────────────────────────────────────────────────────

/// Used to supply computed vectors for a lazy vector index.
///
/// Implements `Iterator<Item = Value>` — each yielded `Value` is the document
/// field expression result for one pending entry. Call `set_vector` or `skip`
/// with the same index before calling `finish`.
pub struct IndexUpdater {
    cbl_ref: *mut CBLIndexUpdater,
    cursor: usize,
}

impl CblRef for IndexUpdater {
    type Output = *mut CBLIndexUpdater;
    fn get_ref(&self) -> Self::Output {
        self.cbl_ref
    }
}

impl IndexUpdater {
    pub(crate) const fn take_ownership(cbl_ref: *mut CBLIndexUpdater) -> Self {
        Self { cbl_ref, cursor: 0 }
    }

    /// Returns the number of vectors that need to be computed.
    ///
    /// Prefer `updater.len()` (from `ExactSizeIterator`) when iterating.
    pub fn pending_count(&self) -> usize {
        unsafe { CBLIndexUpdater_Count(self.cbl_ref) }
    }

    /// Returns the Fleece value at `index` whose vector needs to be computed.
    pub fn value(&self, index: usize) -> Value {
        Value { cbl_ref: unsafe { CBLIndexUpdater_Value(self.cbl_ref, index) } }
    }

    /// Sets the computed vector for the entry at `index`.
    pub fn set_vector(&self, index: usize, vector: &[f32]) -> Result<()> {
        unsafe {
            let mut err = CBLError::default();
            let ok = CBLIndexUpdater_SetVector(
                self.cbl_ref,
                index,
                vector.as_ptr(),
                vector.len(),
                &mut err,
            );
            if ok {
                Ok(())
            } else {
                failure(err)
            }
        }
    }

    /// Marks the entry at `index` as skipped (vector will be recomputed next time).
    pub fn skip(&self, index: usize) {
        unsafe { CBLIndexUpdater_SkipVector(self.cbl_ref, index) }
    }

    /// Commits all set vectors to the index.
    ///
    /// The updater must not be used after this call.
    pub fn finish(self) -> Result<()> {
        unsafe {
            let mut err = CBLError::default();
            let ok = CBLIndexUpdater_Finish(self.cbl_ref, &mut err);
            // Prevent Drop from double-releasing — the updater is consumed.
            std::mem::forget(self);
            if ok {
                Ok(())
            } else {
                failure(err)
            }
        }
    }
}

impl Drop for IndexUpdater {
    fn drop(&mut self) {
        unsafe { release(self.cbl_ref) }
    }
}

/// Iterates over the values that need vectors computed.
///
/// Each item corresponds to the document field expression result for one
/// pending entry. The iterator index matches the `index` parameter expected
/// by `set_vector` and `skip`.
impl Iterator for IndexUpdater {
    type Item = Value;

    fn next(&mut self) -> Option<Self::Item> {
        if self.cursor >= self.pending_count() {
            return None;
        }
        let raw = unsafe { CBLIndexUpdater_Value(self.cbl_ref, self.cursor) };
        self.cursor += 1;
        // Value is a borrowed view — it does not own the pointer and has no Drop.
        Some(Value { cbl_ref: raw })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.pending_count().saturating_sub(self.cursor);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for IndexUpdater {}

// ── QueryIndex::begin_update ──────────────────────────────────────────────────

impl QueryIndex {
    /// Returns an `IndexUpdater` for a lazy vector index, or `None` if the index
    /// is already up-to-date.
    pub fn begin_update(&self, limit: usize) -> Result<Option<IndexUpdater>> {
        unsafe {
            let mut err = CBLError::default();
            let ptr = CBLQueryIndex_BeginUpdate(self.get_ref(), limit, &mut err);
            if !err {
                if ptr.is_null() {
                    Ok(None)
                } else {
                    Ok(Some(IndexUpdater::take_ownership(ptr)))
                }
            } else {
                failure(err)
            }
        }
    }
}
