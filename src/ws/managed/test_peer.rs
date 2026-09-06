use tokio::sync::mpsc;

use crate::ws::types::ClientMessage;

use super::ManagedCommand;

/// Test peer for acknowledging managed WebSocket commands.
pub struct ManagedWsTestPeer {
    commands: mpsc::Receiver<ManagedCommand>,
}

impl ManagedWsTestPeer {
    #[cfg(not(test))]
    pub(super) const fn new(commands: mpsc::Receiver<ManagedCommand>) -> Self {
        Self { commands }
    }

    /// Acknowledge and return the next send command.
    pub async fn acknowledge_next_send(&mut self) -> Option<ClientMessage> {
        match self.commands.recv().await? {
            ManagedCommand::Send { message, tx, .. } => {
                let _ = tx.send(Ok(()));
                serde_json::from_str(message.as_str()).ok()
            }
            _ => None,
        }
    }
}
