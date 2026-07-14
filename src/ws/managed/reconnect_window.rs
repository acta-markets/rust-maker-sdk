use std::time::Duration;

use tokio::sync::{broadcast, mpsc};
use tokio::time::sleep;

use super::{ManagedCommand, ManagedWsError, ManagedWsEvent, SendAwaitError, send_event};

pub(super) async fn wait_reconnect_window(
    cmd_rx: &mut mpsc::Receiver<ManagedCommand>,
    cancel_rx: &mut mpsc::UnboundedReceiver<u64>,
    delay: Duration,
    events_tx: &broadcast::Sender<ManagedWsEvent>,
    next_attempt: u64,
) -> bool {
    send_event(
        events_tx,
        ManagedWsEvent::Reconnecting {
            attempt: next_attempt,
            delay_ms: delay.as_millis() as u64,
        },
    );

    let sleeper = sleep(delay);
    tokio::pin!(sleeper);

    loop {
        tokio::select! {
            _ = &mut sleeper => return false,
            _ = cancel_rx.recv() => {}
            maybe_cmd = cmd_rx.recv() => {
                match maybe_cmd {
                    Some(ManagedCommand::Send { tx, .. }) => {
                        let _ = tx.send(Err(ManagedWsError::Disconnected));
                    }
                    Some(ManagedCommand::SendAwait { tx, .. }) => {
                        let _ = tx.send(Err(SendAwaitError::Disconnected));
                    }
                    Some(ManagedCommand::Close { tx }) => {
                        let _ = tx.send(());
                        return true;
                    }
                    None => return true,
                }
            }
        }
    }
}
