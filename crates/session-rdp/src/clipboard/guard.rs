//! A `CLIPRDR` channel that survives a malformed PDU.
//!
//! `ironrdp-session` ends the whole session when a channel processor returns an error. A
//! hostile or buggy server must not be able to end the desktop by sending a broken clipboard
//! PDU, so errors are absorbed here: logged without any content, counted, and after
//! [`MAX_ERRORS`] of them the channel stops answering (the clipboard is then simply off).

use ironrdp_cliprdr::CliprdrClient;
use ironrdp_core::impl_as_any;
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::PduResult;
use ironrdp_svc::{CompressionCondition, SvcClientProcessor, SvcMessage, SvcProcessor};

/// Malformed PDUs tolerated before the clipboard is switched off for the session.
pub const MAX_ERRORS: u32 = 8;

#[derive(Debug)]
pub struct GuardedCliprdr {
    inner: CliprdrClient,
    errors: u32,
}

impl GuardedCliprdr {
    pub fn new(inner: CliprdrClient) -> Self {
        Self { inner, errors: 0 }
    }

    pub fn inner_mut(&mut self) -> &mut CliprdrClient {
        &mut self.inner
    }

    /// Malformed PDUs seen so far.
    #[cfg(test)]
    pub fn errors(&self) -> u32 {
        self.errors
    }

    /// Too many malformed PDUs: the channel ignores everything from now on.
    pub fn is_disabled(&self) -> bool {
        self.errors >= MAX_ERRORS
    }
}

impl_as_any!(GuardedCliprdr);

impl SvcProcessor for GuardedCliprdr {
    fn channel_name(&self) -> ChannelName {
        self.inner.channel_name()
    }

    fn compression_condition(&self) -> CompressionCondition {
        self.inner.compression_condition()
    }

    fn start(&mut self) -> PduResult<Vec<SvcMessage>> {
        self.inner.start()
    }

    fn process(&mut self, payload: &[u8]) -> PduResult<Vec<SvcMessage>> {
        if self.is_disabled() {
            return Ok(Vec::new());
        }
        match self.inner.process(payload) {
            Ok(messages) => Ok(messages),
            Err(e) => {
                self.errors += 1;
                // The error names the kind of problem, never the clipboard data.
                tracing::warn!(
                    errors = self.errors,
                    kind = e.kind().to_string(),
                    "ignoring a malformed clipboard PDU"
                );
                Ok(Vec::new())
            }
        }
    }
}

impl SvcClientProcessor for GuardedCliprdr {}
