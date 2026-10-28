use super::{InnerState, TransferState};
use std::time::Duration;
use tokio::time::Instant;

pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSFER_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Channel traffic and keep-alives must not prolong an incomplete handshake or
/// a file transfer that makes no payload progress. Consent has no app deadline.
pub(crate) struct TransferDeadline {
    handshake: Instant,
    transfer: Option<(u64, Instant)>,
}

impl TransferDeadline {
    pub fn new(handshake_timeout: Duration) -> Self {
        Self {
            handshake: Instant::now() + handshake_timeout,
            transfer: None,
        }
    }

    pub fn next(&mut self, state: &InnerState) -> Option<Instant> {
        match state.state {
            TransferState::WaitingForUserConsent | TransferState::SentIntroduction => None,
            TransferState::ReceivingFiles | TransferState::SendingFiles => {
                let acknowledged = state
                    .transfer_metadata
                    .as_ref()
                    .map(|m| m.ack_bytes)
                    .unwrap_or(0);
                let (previous, deadline) = self
                    .transfer
                    .get_or_insert_with(|| (acknowledged, Instant::now() + TRANSFER_IDLE_TIMEOUT));
                if acknowledged > *previous {
                    *previous = acknowledged;
                    *deadline = Instant::now() + TRANSFER_IDLE_TIMEOUT;
                }
                Some(*deadline)
            }
            TransferState::Finished | TransferState::Cancelled | TransferState::Rejected => None,
            _ => Some(self.handshake),
        }
    }
}

pub(crate) async fn wait(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hdl::info::{TransferMetadata, TransferPayloadKind};

    #[test]
    fn handshake_deadline_is_absolute_and_consent_has_no_deadline() {
        let mut budget = TransferDeadline::new(HANDSHAKE_TIMEOUT);
        let mut state = InnerState::default();
        let initial = budget.next(&state).unwrap();
        for stage in [
            TransferState::ReceivedConnectionRequest,
            TransferState::SentUkeyClientInit,
            TransferState::SentPairedKeyResult,
        ] {
            state.state = stage;
            assert_eq!(budget.next(&state), Some(initial));
        }
        for stage in [
            TransferState::SentIntroduction,
            TransferState::WaitingForUserConsent,
        ] {
            state.state = stage;
            assert_eq!(budget.next(&state), None);
        }
        state.state = TransferState::ReceivingFiles;
        assert!(budget.next(&state).unwrap() > initial);
    }

    #[test]
    fn transfer_deadline_only_resets_for_payload_progress() {
        let mut budget = TransferDeadline::new(HANDSHAKE_TIMEOUT);
        let mut state = InnerState {
            state: TransferState::ReceivingFiles,
            transfer_metadata: Some(TransferMetadata {
                id: "test".into(),
                source: None,
                pin_code: None,
                payload_kind: TransferPayloadKind::Files,
                payload_preview: None,
                payload: None,
                total_bytes: 100,
                ack_bytes: 0,
            }),
            ..Default::default()
        };
        let first = budget.next(&state).unwrap();
        assert_eq!(budget.next(&state), Some(first));
        let older = first - Duration::from_secs(10);
        budget.transfer = Some((0, older));
        assert_eq!(budget.next(&state), Some(older));
        state.transfer_metadata.as_mut().unwrap().ack_bytes = 1;
        assert!(budget.next(&state).unwrap() > older);
        let progressed = budget.next(&state);
        state.transfer_metadata.as_mut().unwrap().ack_bytes = 0;
        assert_eq!(budget.next(&state), progressed);
    }
}
