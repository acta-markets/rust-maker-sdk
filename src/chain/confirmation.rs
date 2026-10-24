use solana_sdk::signature::Signature;

use super::{ChainError, rpc::ChainClient};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmationStatus {
    Confirmed,
    Failed(String),
    /// Not observed at the requested commitment. Never proves failure or expiry.
    Pending,
}

impl ChainClient {
    /// One status read per chunk of at most 256 signatures, including history.
    pub async fn signature_statuses(
        &self,
        signatures: &[Signature],
    ) -> Result<Vec<ConfirmationStatus>, ChainError> {
        let mut results = Vec::with_capacity(signatures.len());
        for chunk in signatures.chunks(256) {
            let statuses = self
                .rpc()
                .get_signature_statuses_with_history(chunk)
                .await?
                .value;
            if statuses.len() != chunk.len() {
                return Err(ChainError::InvalidAccountData);
            }
            results.extend(statuses.into_iter().map(|status| match status {
                Some(status) if status.satisfies_commitment(self.rpc().commitment()) => {
                    match status.err {
                        Some(error) => ConfirmationStatus::Failed(error.to_string()),
                        None => ConfirmationStatus::Confirmed,
                    }
                }
                _ => ConfirmationStatus::Pending,
            }));
        }
        Ok(results)
    }

    /// Wait on one Solana WS connection, then reconcile unresolved signatures in one
    /// chunked status pass on disconnect, subscription error, or deadline. No polling.
    #[cfg(feature = "ws-client")]
    pub async fn confirm_signatures(
        &self,
        ws_url: &str,
        signatures: &[Signature],
        timeout: std::time::Duration,
    ) -> Result<Vec<ConfirmationStatus>, ChainError> {
        let mut results = vec![ConfirmationStatus::Pending; signatures.len()];
        if signatures.is_empty() {
            return Ok(results);
        }
        let _ = tokio::time::timeout(
            timeout,
            self.subscribe_signatures(ws_url, signatures, &mut results),
        )
        .await;
        let unresolved: Vec<_> = results
            .iter()
            .enumerate()
            .filter_map(|(i, status)| (*status == ConfirmationStatus::Pending).then_some(i))
            .collect();
        let pending: Vec<_> = unresolved.iter().map(|i| signatures[*i]).collect();
        for (index, status) in unresolved
            .into_iter()
            .zip(self.signature_statuses(&pending).await?)
        {
            results[index] = status;
        }
        Ok(results)
    }

    #[cfg(feature = "ws-client")]
    async fn subscribe_signatures(
        &self,
        ws_url: &str,
        signatures: &[Signature],
        results: &mut [ConfirmationStatus],
    ) -> Result<(), ChainError> {
        use futures_util::{SinkExt, StreamExt};
        use serde_json::{Value, json};
        use tokio_tungstenite::{connect_async, tungstenite::Message};
        let transport = |error: tokio_tungstenite::tungstenite::Error| {
            ChainError::Confirmation(error.to_string())
        };
        let (mut socket, _) = connect_async(ws_url).await.map_err(transport)?;
        for (index, signature) in signatures.iter().enumerate() {
            socket.send(Message::Text(json!({"jsonrpc":"2.0", "id":index, "method":"signatureSubscribe", "params":[signature.to_string(), {"commitment": self.rpc().commitment().commitment}]}).to_string().into())).await.map_err(transport)?;
        }
        let mut subscriptions = std::collections::BTreeMap::new();
        let mut early = std::collections::BTreeMap::new();
        while let Some(message) = socket.next().await {
            let message = message.map_err(transport)?;
            let value: Value = match message {
                Message::Text(text) => serde_json::from_str(&text)
                    .map_err(|error| ChainError::Confirmation(error.to_string()))?,
                Message::Ping(data) => {
                    socket.send(Message::Pong(data)).await.map_err(transport)?;
                    continue;
                }
                Message::Close(_) => break,
                _ => continue,
            };
            if value.get("error").is_some() {
                break;
            }
            if let (Some(id), Some(subscription)) = (value["id"].as_u64(), value["result"].as_u64())
            {
                let index = usize::try_from(id).map_err(|_| ChainError::InvalidAccountData)?;
                if index >= signatures.len() {
                    return Err(ChainError::InvalidAccountData);
                }
                if let Some(status) = early.remove(&subscription) {
                    results[index] = status;
                } else {
                    subscriptions.insert(subscription, index);
                }
            } else if value["method"] == "signatureNotification" {
                let subscription = value["params"]["subscription"]
                    .as_u64()
                    .ok_or(ChainError::InvalidAccountData)?;
                let error = value["params"]["result"]["value"]
                    .get("err")
                    .ok_or(ChainError::InvalidAccountData)?;
                let status = if error.is_null() {
                    ConfirmationStatus::Confirmed
                } else {
                    ConfirmationStatus::Failed(error.to_string())
                };
                if let Some(index) = subscriptions.remove(&subscription) {
                    results[index] = status;
                } else {
                    if early.len() >= signatures.len() {
                        return Err(ChainError::InvalidAccountData);
                    }
                    early.insert(subscription, status);
                }
            }
            if results
                .iter()
                .all(|status| *status != ConfirmationStatus::Pending)
            {
                break;
            }
        }
        Ok(())
    }
}

#[cfg(all(test, feature = "ws-client"))]
#[path = "confirmation_tests.rs"]
mod tests;
