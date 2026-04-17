// Predictive query API — enterprise edition only.
//
// Wraps CBLPredictiveModel, CBL_RegisterPredictiveModel, CBL_UnregisterPredictiveModel.

use crate::{
    CblRef,
    c_api::{
        CBLPredictiveModel, CBL_RegisterPredictiveModel, CBL_UnregisterPredictiveModel, FLDict,
        FLMutableDict,
    },
    fleece::Dict,
    fleece_mutable::MutableDict,
    slice::from_str,
};

// ── PredictiveModel trait ─────────────────────────────────────────────────────

/// A model that can be invoked from a N1QL `PREDICTION()` expression.
///
/// Implementations must be `Send + Sync` because CBL may call `predict` from
/// any thread.
pub trait PredictiveModel: Send + Sync {
    /// Called with the input dictionary from the query.
    ///
    /// Return `Some(output)` to provide a result, or `None` to produce no output.
    fn predict(&self, input: Dict) -> Option<MutableDict>;
}

// ── register_predictive_model ─────────────────────────────────────────────────

/// Registers a predictive model under the given name.
///
/// The model is boxed and kept alive until `unregister_predictive_model` is
/// called, at which point CBL invokes the `unregistered` callback and the box
/// is dropped.
pub fn register_predictive_model(name: &str, model: impl PredictiveModel + 'static) {
    // Double-box so we can pass a thin `*mut c_void` context pointer.
    let boxed: Box<Box<dyn PredictiveModel>> = Box::new(Box::new(model));
    let ctx = Box::into_raw(boxed) as *mut std::ffi::c_void;

    let cbl_model = CBLPredictiveModel {
        context: ctx,
        prediction: Some(c_prediction),
        unregistered: Some(c_unregistered),
    };

    unsafe {
        CBL_RegisterPredictiveModel(from_str(name).get_ref(), cbl_model);
    }
}

/// Unregisters the predictive model with the given name.
///
/// CBL will call the `unregistered` callback, which drops the boxed model.
pub fn unregister_predictive_model(name: &str) {
    unsafe {
        CBL_UnregisterPredictiveModel(from_str(name).get_ref());
    }
}

// ── C callbacks ───────────────────────────────────────────────────────────────

unsafe extern "C" fn c_prediction(context: *mut std::ffi::c_void, input: FLDict) -> FLMutableDict {
    unsafe {
        let model = &*(context as *const Box<dyn PredictiveModel>);
        // Wrap the input FLDict as a borrowed Dict. The lifetime is tied to the
        // callback invocation, which is safe because CBL keeps the dict alive.
        let dict = Dict::wrap(input, &input);
        match model.predict(dict) {
            Some(result) => {
                // Transfer ownership to CBL — CBL releases the dict after the call.
                let raw = result.get_ref();
                // Prevent MutableDict's Drop from releasing the pointer; CBL owns it now.
                std::mem::forget(result);
                raw
            }
            None => std::ptr::null_mut(),
        }
    }
}

unsafe extern "C" fn c_unregistered(context: *mut std::ffi::c_void) {
    unsafe {
        // Reconstruct and drop the box, freeing the model.
        let _ = Box::from_raw(context as *mut Box<dyn PredictiveModel>);
    }
}
