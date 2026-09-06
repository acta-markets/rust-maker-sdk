use std::time::Duration;

use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::sleep;

use super::{
    ManagedCommand, ManagedWsError, ManagedWsEvent, ManagedWsState, SendAwaitError,
    transition_state,
};

pub(super) async fn wait_reconnect_window(
    cmd_rx: &mut mpsc::Receiver<ManagedCommand>,
    cancel_rx: &mut mpsc::UnboundedReceiver<u64>,
    delay: Duration,
    events_tx: &broadcast::Sender<ManagedWsEvent>,
    next_attempt: u64,
    state_tx: &watch::Sender<ManagedWsState>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> bool {
    let delay_ms = delay.as_millis() as u64;
    transition_state(
        state_tx,
        events_tx,
        ManagedWsState::Reconnecting {
            attempt: next_attempt,
            delay_ms,
        },
        ManagedWsEvent::Reconnecting {
            attempt: next_attempt,
            delay_ms,
        },
    );

    let sleeper = sleep(delay);
    tokio::pin!(sleeper);

    loop {
        tokio::select! {
            biased;
            _ = wait_for_shutdown(shutdown_rx) => return true,
            _ = &mut sleeper => return false,
            maybe_await_id = cancel_rx.recv() => {
                if maybe_await_id.is_none() {
                    return true;
                }
            }
            maybe_cmd = cmd_rx.recv() => {
                match maybe_cmd {
                    Some(ManagedCommand::Send { tx, .. }) => {
                        let _ = tx.send(Err(ManagedWsError::Disconnected));
                    }
                    Some(ManagedCommand::SendAwait { tx, .. }) => {
                        let _ = tx.send(Err(SendAwaitError::Disconnected));
                    }
                    None => return true,
                }
            }
        }
    }
}

async fn wait_for_shutdown(shutdown_rx: &mut watch::Receiver<bool>) {
    if *shutdown_rx.borrow() {
        return;
    }
    let _ = shutdown_rx.changed().await;
}
