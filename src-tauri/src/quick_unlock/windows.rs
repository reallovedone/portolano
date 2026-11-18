//! Windows Hello through `UserConsentVerifier` (fingerprint, face or the Hello PIN). With the
//! main window as owner the prompt appears in front of it (`IUserConsentVerifierInterop`).
//! Works without app signing.

use std::ffi::c_void;
use std::sync::mpsc;
use std::time::Duration;

use windows::core::{factory, HSTRING};
use windows::Security::Credentials::UI::{
    UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
};
use windows::Win32::Foundation::{HWND, RPC_E_CHANGED_MODE};
use windows::Win32::System::WinRT::{
    IUserConsentVerifierInterop, RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED,
};
use windows_future::IAsyncOperation;

use super::{wait_for_reply, BiometricVerifier, VerifyError, PROMPT_TIMEOUT, REASON};

/// Asking the system whether Hello is set up is quick; do not wait longer than this.
const AVAILABILITY_TIMEOUT: Duration = Duration::from_secs(10);

/// Keeps the calling thread inside a WinRT apartment. The commands run on a tokio blocking-pool
/// thread, which has none, and `RoGetActivationFactory` would fail with `CO_E_NOTINITIALIZED`.
struct Apartment(bool);

impl Apartment {
    fn enter() -> Self {
        // SAFETY: plain apartment initialisation; balanced by `RoUninitialize` in `Drop` only
        // when this call succeeded. `S_FALSE` (already initialised) is a success that must be
        // balanced too; `RPC_E_CHANGED_MODE` (the thread is already in another apartment) is
        // an error: the thread is usable and nothing is balanced.
        match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            Ok(()) => Self(true),
            Err(e) => {
                if e.code() != RPC_E_CHANGED_MODE {
                    tracing::warn!(error = %e, "could not initialise the WinRT apartment");
                }
                Self(false)
            }
        }
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful `RoInitialize` of `enter` on this same thread.
            unsafe { RoUninitialize() };
        }
    }
}

pub struct SystemVerifier;

impl BiometricVerifier for SystemVerifier {
    fn available(&self) -> bool {
        let _apartment = Apartment::enter();
        // The wait is bounded: a Hello stack that never answers must not block the UI's query.
        let (tx, rx) = mpsc::channel();
        let started = UserConsentVerifier::CheckAvailabilityAsync().and_then(|op| {
            op.when(move |r| {
                if tx.send(r).is_err() {
                    tracing::debug!("availability answer arrived after the wait ended");
                }
            })?;
            Ok(op)
        });
        let op = match started {
            Ok(op) => op,
            Err(e) => {
                tracing::warn!(error = %e, "Windows Hello availability could not be read");
                return false;
            }
        };
        match rx.recv_timeout(AVAILABILITY_TIMEOUT) {
            Ok(Ok(a)) => a == UserConsentVerifierAvailability::Available,
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "Windows Hello availability could not be read");
                false
            }
            Err(_) => {
                tracing::warn!("Windows Hello availability timed out");
                cancel(&op);
                false
            }
        }
    }

    fn verify(&self, owner: Option<isize>) -> Result<(), VerifyError> {
        let _apartment = Apartment::enter();
        let message = HSTRING::from(REASON);
        let started = match owner {
            Some(hwnd) => {
                factory::<UserConsentVerifier, IUserConsentVerifierInterop>().and_then(|interop| {
                    // SAFETY: `hwnd` is the handle of the live main window, taken by the caller
                    // just before; the message outlives the call.
                    unsafe {
                        interop.RequestVerificationForWindowAsync::<
                            IAsyncOperation<UserConsentVerificationResult>,
                        >(HWND(hwnd as *mut c_void), &message)
                    }
                })
            }
            None => UserConsentVerifier::RequestVerificationAsync(&message),
        };
        let (tx, rx) = mpsc::channel::<Result<(), VerifyError>>();
        let op = match started.and_then(|op| {
            op.when(move |r| {
                let outcome = match r {
                    Ok(result) => classify(result),
                    Err(e) => {
                        tracing::debug!(error = %e, "Windows Hello request failed");
                        Err(VerifyError::Failed)
                    }
                };
                if tx.send(outcome).is_err() {
                    tracing::debug!("Windows Hello answer arrived after the wait ended");
                }
            })?;
            Ok(op)
        }) {
            Ok(op) => op,
            Err(e) => {
                tracing::debug!(error = %e, "Windows Hello request could not start");
                return Err(VerifyError::Failed);
            }
        };
        // Bounded wait; on timeout the request is cancelled.
        wait_for_reply(&rx, PROMPT_TIMEOUT, || cancel(&op))
    }
}

/// Cancels a request that did not answer in time.
fn cancel<T: windows::core::RuntimeType + 'static>(op: &IAsyncOperation<T>) {
    if let Err(e) = op.Cancel() {
        tracing::debug!(error = %e, "could not cancel the Windows Hello request");
    }
}

fn classify(result: UserConsentVerificationResult) -> Result<(), VerifyError> {
    if result == UserConsentVerificationResult::Verified {
        Ok(())
    } else if result == UserConsentVerificationResult::Canceled {
        Err(VerifyError::Cancelled)
    } else {
        Err(VerifyError::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_verified_succeeds_and_cancel_is_a_cancellation() {
        assert_eq!(classify(UserConsentVerificationResult::Verified), Ok(()));
        assert_eq!(
            classify(UserConsentVerificationResult::Canceled),
            Err(VerifyError::Cancelled)
        );
        for r in [
            UserConsentVerificationResult::DeviceNotPresent,
            UserConsentVerificationResult::NotConfiguredForUser,
            UserConsentVerificationResult::DisabledByPolicy,
            UserConsentVerificationResult::DeviceBusy,
            UserConsentVerificationResult::RetriesExhausted,
        ] {
            assert_eq!(classify(r), Err(VerifyError::Failed));
        }
    }
}
