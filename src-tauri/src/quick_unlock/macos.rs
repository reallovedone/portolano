//! Touch ID through LocalAuthentication (`LAContext`). Biometrics only: the policy is
//! `LAPolicyDeviceOwnerAuthenticationWithBiometrics` and the "Use Password..." button is hidden,
//! because the fallback is Portolano's own master password, not the password of the Mac.
//! Works without app signing.

use std::sync::mpsc;

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_local_authentication::{LAContext, LAError, LAPolicy};

use super::{wait_for_reply, BiometricVerifier, VerifyError, PROMPT_TIMEOUT, REASON};

const POLICY: LAPolicy = LAPolicy::DeviceOwnerAuthenticationWithBiometrics;

pub struct SystemVerifier;

impl BiometricVerifier for SystemVerifier {
    fn available(&self) -> bool {
        // SAFETY: a fresh context used only on this thread; the call has no other requirement.
        let context = unsafe { LAContext::new() };
        match unsafe { context.canEvaluatePolicy_error(POLICY) } {
            Ok(()) => true,
            Err(e) => {
                tracing::debug!(code = e.code(), "Touch ID is not available");
                false
            }
        }
    }

    fn verify(&self, _owner: Option<isize>) -> Result<(), VerifyError> {
        // SAFETY: a fresh context; the reply block only sends on a channel (it is `Send`), and
        // the context stays alive until the reply arrived (we wait for it below).
        let context = unsafe { LAContext::new() };
        // An empty title hides the "Use Password..." button.
        unsafe { context.setLocalizedFallbackTitle(Some(&NSString::from_str(""))) };
        let (tx, rx) = mpsc::channel::<Result<(), VerifyError>>();
        let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
            let outcome = if success.as_bool() {
                Ok(())
            } else {
                // SAFETY: LocalAuthentication passes either null or a valid NSError.
                match unsafe { error.as_ref() } {
                    Some(e) => Err(classify(e.code())),
                    None => Err(VerifyError::Failed),
                }
            };
            if tx.send(outcome).is_err() {
                tracing::debug!("Touch ID reply arrived after the request was dropped");
            }
        });
        unsafe {
            context.evaluatePolicy_localizedReason_reply(
                POLICY,
                &NSString::from_str(REASON),
                &reply,
            );
        }
        // A prompt that never answers must not hold the request forever: after the timeout the
        // context is invalidated (the system dismisses the dialog) and the check has failed.
        wait_for_reply(&rx, PROMPT_TIMEOUT, || {
            // SAFETY: `invalidate` may be called from any thread on a live context.
            unsafe { context.invalidate() };
        })
    }
}

/// Maps a `LAError` code: the user's own "no" is a cancellation, anything else a failure.
fn classify(code: isize) -> VerifyError {
    let code = LAError(code);
    if code == LAError::UserCancel
        || code == LAError::UserFallback
        || code == LAError::SystemCancel
        || code == LAError::AppCancel
    {
        VerifyError::Cancelled
    } else {
        VerifyError::Failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_users_refusals_are_cancellations_and_the_rest_failures() {
        for code in [-2, -3, -4, -9] {
            assert_eq!(classify(code), VerifyError::Cancelled, "{code}");
        }
        for code in [-1, -5, -6, -7, -8, -10, 0] {
            assert_eq!(classify(code), VerifyError::Failed, "{code}");
        }
    }
}
