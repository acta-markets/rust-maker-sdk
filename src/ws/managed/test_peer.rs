use tokio::sync::mpsc;

use crate::ws::types::ClientMessage;

use super::ManagedCommand;

/// Opaque peer for acknowledging managed WebSocket commands in downstream tests.
pub struct ManagedWsTestPeer {
    commands: mpsc::Receiver<ManagedCommand>,
}

impl ManagedWsTestPeer {
    #[cfg(not(test))]
    pub(super) const fn new(commands: mpsc::Receiver<ManagedCommand>) -> Self {
        Self { commands }
    }

    /// Receive and acknowledge one ordinary send command.
    pub async fn acknowledge_next_send(&mut self) -> Option<ClientMessage> {
        match self.commands.recv().await? {
            ManagedCommand::Send { message, tx } => {
                let _ = tx.send(Ok(()));
                Some(message)
            }
            _ => None,
        }
    }
}
