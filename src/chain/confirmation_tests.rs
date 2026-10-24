use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use solana_commitment_config::CommitmentConfig;
use solana_rpc_client::{
    api::{client_error, request::RpcRequest},
    nonblocking::rpc_client::RpcClient,
    rpc_client::RpcClientConfig,
    rpc_sender::{RpcSender, RpcTransportStats},
};
use std::sync::{Arc, Mutex};

struct Statuses(Arc<Mutex<Vec<usize>>>);
#[async_trait::async_trait]
impl RpcSender for Statuses {
    async fn send(&self, request: RpcRequest, params: Value) -> client_error::Result<Value> {
        assert_eq!(request, RpcRequest::GetSignatureStatuses);
        assert_eq!(params[1]["searchTransactionHistory"], true);
        let count = params[0].as_array().unwrap().len();
        assert!(count <= 256);
        self.0.lock().unwrap().push(count);
        let statuses: Vec<_> = (0..count).map(|_| json!({"slot":1,"confirmations":1,"err":null,"status":{"Ok":null},"confirmationStatus":"confirmed"})).collect();
        Ok(json!({"context":{"slot":1},"value":statuses}))
    }
    fn get_transport_stats(&self) -> RpcTransportStats {
        RpcTransportStats::default()
    }
    fn url(&self) -> String {
        "mock://statuses".into()
    }
}
fn client() -> (ChainClient, Arc<Mutex<Vec<usize>>>) {
    let calls = Arc::new(Mutex::new(vec![]));
    let rpc = RpcClient::new_sender(
        Statuses(calls.clone()),
        RpcClientConfig::with_commitment(CommitmentConfig::confirmed()),
    );
    (
        ChainClient::from_rpc(rpc, solana_sdk::pubkey::Pubkey::new_unique()),
        calls,
    )
}

#[tokio::test]
async fn status_fallback_chunks_257_and_preserves_order() {
    let (client, calls) = client();
    let signatures = vec![Signature::default(); 257];
    let results = client
        .confirm_signatures(
            "invalid://endpoint",
            &signatures,
            std::time::Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert_eq!(results, vec![ConfirmationStatus::Confirmed; 257]);
    assert_eq!(*calls.lock().unwrap(), vec![256, 1]);
}

#[tokio::test]
async fn notification_before_ack_preserves_failure_and_falls_back_only_for_unresolved() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        for i in 0..2 {
            let message = ws.next().await.unwrap().unwrap();
            let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            assert_eq!(request["method"], "signatureSubscribe");
            assert_eq!(request["params"][1]["commitment"], "confirmed");
            if i == 0 {
                ws.send(tokio_tungstenite::tungstenite::Message::Text(json!({"jsonrpc":"2.0","method":"signatureNotification","params":{"subscription":10,"result":{"context":{"slot":1},"value":{"err":"AccountNotFound"}}}}).to_string().into())).await.unwrap();
            }
            ws.send(tokio_tungstenite::tungstenite::Message::Text(
                json!({"jsonrpc":"2.0","id":i,"result":i+10})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        }
        ws.close(None).await.unwrap();
    });
    let (client, calls) = client();
    let result = client
        .confirm_signatures(
            &url,
            &[Signature::default(); 2],
            std::time::Duration::from_secs(2),
        )
        .await
        .unwrap();
    assert!(matches!(result[0], ConfirmationStatus::Failed(_)));
    assert_eq!(result[1], ConfirmationStatus::Confirmed);
    assert_eq!(*calls.lock().unwrap(), vec![1]);
    server.await.unwrap();
}
