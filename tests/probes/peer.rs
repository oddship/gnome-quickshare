//! A real wire-protocol peer for driving consent UI checks. Not shipped by default.
use rqs_lib::channel::TransferKind;
use rqs_lib::sharing_nearby::wifi_credentials_metadata::SecurityType;
use rqs_lib::{OutboundPayload, RQS, SendInfo, TransferState, Visibility, WifiPayload};
use std::time::Duration;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert!(
        args.len() == 3,
        "Usage: quickshare-probe ADDRESS FILE|--wifi"
    );
    let address = args[1].clone();
    let mut service = RQS::new(
        Visibility::Invisible,
        None,
        None,
        Some("Quickshare test peer".into()),
    );
    let mut messages = service.message_sender.subscribe();
    let (sender, _) = service.run().await.expect("start test peer");
    sender
        .send(SendInfo {
            id: address.clone(),
            addr: address,
            name: "GNOME Quickshare".into(),
            ob: if args[2] == "--wifi" {
                OutboundPayload::Wifi(WifiPayload {
                    ssid: "Quickshare test network".into(),
                    password: "not-a-real-password".into(),
                    security_type: SecurityType::WpaPsk,
                    hidden_ssid: true,
                })
            } else {
                OutboundPayload::Files(vec![args[2].clone()])
            },
        })
        .await
        .expect("queue file");
    loop {
        let message = tokio::time::timeout(Duration::from_secs(45), messages.recv())
            .await
            .expect("peer timed out")
            .expect("peer channel closed");
        let Some(client) = message.msg.as_client() else {
            continue;
        };
        if client.kind != TransferKind::Outbound {
            continue;
        }
        if let Some(metadata) = &client.metadata
            && let Some(code) = &metadata.pin_code
        {
            println!("Authentication code: {code}");
        }
        if matches!(
            client.state,
            Some(
                TransferState::Finished
                    | TransferState::Rejected
                    | TransferState::Cancelled
                    | TransferState::Disconnected
            )
        ) {
            println!("Outcome: {:?}", client.state);
            assert_eq!(client.state, Some(TransferState::Finished));
            break;
        }
    }
    service.stop().await;
}
