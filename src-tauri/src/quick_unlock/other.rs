//! Systems without a supported biometric method: quick unlock is "not available".

use super::{BiometricVerifier, VerifyError};

pub struct SystemVerifier;

impl BiometricVerifier for SystemVerifier {
    fn available(&self) -> bool {
        false
    }

    fn verify(&self, _owner: Option<isize>) -> Result<(), VerifyError> {
        Err(VerifyError::Failed)
    }
}
