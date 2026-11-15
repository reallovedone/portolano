//! `AppError` at the IPC boundary. The type is defined in `portolano-core-model` (the session
//! crates embed it in their channel events); this module is where `src-tauri` code imports it
//! from and where conversions that only make sense in the app live.

pub use portolano_core_model::AppError;
#[allow(unused_imports)] // re-exported for the command groups that will need it
pub use portolano_core_model::ErrorCode;

use std::sync::PoisonError;

/// A poisoned lock means another command panicked while holding it.
pub fn poisoned<T>(_: PoisonError<T>) -> AppError {
    AppError::internal("internal state is poisoned")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[test]
    fn poisoned_lock_maps_to_internal() {
        let mutex = std::sync::Arc::new(Mutex::new(0));
        let clone = mutex.clone();
        let _ = std::thread::spawn(move || {
            let _guard = clone.lock();
            panic!("poison the mutex");
        })
        .join();
        let err = mutex
            .lock()
            .map(|_| ())
            .map_err(poisoned)
            .expect_err("poisoned");
        assert_eq!(err.code, ErrorCode::Internal);
    }

    #[test]
    fn serialized_shape_is_code_and_message() {
        let json = serde_json::to_value(AppError::unsupported("x")).expect("serialize");
        assert_eq!(json["code"], "unsupported");
        assert!(json["message"].is_string());
    }
}
