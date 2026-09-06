use std::time::Duration;

use futures_util::{Sink, SinkExt};
use tokio::sync::mpsc::{self, error::TrySendError};
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Instant, timeout};
use tokio_tungstenite::tungstenite::{Bytes, Message};

use super::{ManagedWsError, ManagedWsEvent, send_event};
use crate::ws::client::PreparedClientMessage;
use crate::ws::error::WsClientError;
use crate::ws::types::Lane;

pub(super) type WriteResult = oneshot::Sender<Result<(), ManagedWsError>>;

pub(super) enum WriterCommand {
    Frame {
        message: PreparedClientMessage,
        done: Option<WriteResult>,
        written: Option<oneshot::Sender<Instant>>,
    },
    Pong(Bytes),
    Close,
}

/// Sender side of the writer task. The control lane is drained before the
/// data lane. Quotes and cancellations share the FIFO data lane.
pub(super) struct WriterLanes {
    control: mpsc::Sender<WriterCommand>,
    data: mpsc::Sender<WriterCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LaneFull {
    /// The lane is at capacity: the socket is not draining.
    Full,
    /// The writer task has ended.
    Closed,
}

impl WriterLanes {
    fn enqueue(&self, lane: Lane, command: WriterCommand) -> Result<(), (LaneFull, WriterCommand)> {
        let tx = match lane {
            Lane::Control => &self.control,
            Lane::Data => &self.data,
        };
        tx.try_send(command).map_err(|error| match error {
            TrySendError::Full(command) => (LaneFull::Full, command),
            TrySendError::Closed(command) => (LaneFull::Closed, command),
        })
    }

    /// On failure the write-result sender comes back so the caller can answer it.
    pub(super) fn frame(
        &self,
        message: PreparedClientMessage,
        done: Option<WriteResult>,
    ) -> Result<(), (LaneFull, Option<WriteResult>)> {
        self.frame_with_receipt(message, done, None)
    }

    pub(super) fn frame_with_receipt(
        &self,
        message: PreparedClientMessage,
        done: Option<WriteResult>,
        written: Option<oneshot::Sender<Instant>>,
    ) -> Result<(), (LaneFull, Option<WriteResult>)> {
        let lane = message.lane();
        self.enqueue(
            lane,
            WriterCommand::Frame {
                message,
                done,
                written,
            },
        )
        .map_err(|(full, command)| match command {
            WriterCommand::Frame { done, .. } => (full, done),
            WriterCommand::Pong(_) | WriterCommand::Close => (full, None),
        })
    }

    pub(super) fn pong(&self, payload: Bytes) -> Result<(), LaneFull> {
        self.enqueue(Lane::Control, WriterCommand::Pong(payload))
            .map_err(|(full, _)| full)
    }

    pub(super) fn close(&self) -> Result<(), LaneFull> {
        self.enqueue(Lane::Control, WriterCommand::Close)
            .map_err(|(full, _)| full)
    }
}

/// The task ends on `Close`, when both lanes are dropped, or on the first
/// failed write; a failure is published as an `Error` event first.
pub(super) fn spawn<S>(
    sink: S,
    write_timeout: Duration,
    capacity: usize,
    events_tx: broadcast::Sender<ManagedWsEvent>,
) -> (WriterLanes, JoinHandle<()>)
where
    S: Sink<Message> + Unpin + Send + 'static,
    S::Error: Into<WsClientError> + Send,
{
    let (control, control_rx) = mpsc::channel(capacity);
    let (data, data_rx) = mpsc::channel(capacity);
    let task = tokio::spawn(run(sink, control_rx, data_rx, write_timeout, events_tx));
    (WriterLanes { control, data }, task)
}

async fn run<S>(
    mut sink: S,
    mut control_rx: mpsc::Receiver<WriterCommand>,
    mut data_rx: mpsc::Receiver<WriterCommand>,
    write_timeout: Duration,
    events_tx: broadcast::Sender<ManagedWsEvent>,
) where
    S: Sink<Message> + Unpin,
    S::Error: Into<WsClientError>,
{
    let (mut control_open, mut data_open) = (true, true);
    loop {
        let next = tokio::select! {
            biased;
            command = control_rx.recv(), if control_open => (Lane::Control, command),
            command = data_rx.recv(), if data_open => (Lane::Data, command),
            else => return,
        };
        let command = match next {
            (Lane::Control, None) => {
                control_open = false;
                continue;
            }
            (Lane::Data, None) => {
                data_open = false;
                continue;
            }
            (_, Some(command)) => command,
        };
        match command {
            WriterCommand::Frame {
                message,
                done,
                written,
            } => {
                let result =
                    write(&mut sink, Message::Text(message.into_text()), write_timeout).await;
                let failed = result.is_err();
                if let Err(error) = &result {
                    send_event(&events_tx, ManagedWsEvent::Error(error.to_string()));
                }
                if !failed && let Some(written) = written {
                    let _ = written.send(Instant::now());
                }
                if let Some(done) = done {
                    let _ = done.send(result);
                }
                if failed {
                    return;
                }
            }
            WriterCommand::Pong(payload) => {
                if let Err(error) = write(&mut sink, Message::Pong(payload), write_timeout).await {
                    send_event(&events_tx, ManagedWsEvent::Error(error.to_string()));
                    return;
                }
            }
            WriterCommand::Close => {
                let _ = timeout(write_timeout, sink.close()).await;
                return;
            }
        }
    }
}

async fn write<S>(
    sink: &mut S,
    message: Message,
    write_timeout: Duration,
) -> Result<(), ManagedWsError>
where
    S: Sink<Message> + Unpin,
    S::Error: Into<WsClientError>,
{
    match timeout(write_timeout, sink.send(message)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(ManagedWsError::Write(error.into())),
        Err(_) => Err(ManagedWsError::WriteTimeout),
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    use super::*;
    use crate::ws::types::{ClientMessage, GetMarketsMessage};
    use uuid::Uuid;

    #[derive(Clone, Default)]
    struct RecordingSink {
        sent: Arc<Mutex<Vec<Message>>>,
    }

    impl Sink<Message> for RecordingSink {
        type Error = WsClientError;

        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn start_send(self: Pin<&mut Self>, item: Message) -> Result<(), Self::Error> {
            self.sent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(item);
            Ok(())
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    struct StalledSink;

    impl Sink<Message> for StalledSink {
        type Error = WsClientError;

        fn poll_ready(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }

        fn start_send(self: Pin<&mut Self>, _: Message) -> Result<(), Self::Error> {
            Ok(())
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }

        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

    fn data_frame() -> PreparedClientMessage {
        PreparedClientMessage::new(&ClientMessage::GetMarkets(GetMarketsMessage {
            request_id: Uuid::nil(),
        }))
        .expect("serialize data frame")
    }

    #[tokio::test]
    async fn write_receipt_excludes_time_waiting_behind_earlier_frames() {
        let (permit_tx, permit_rx) = mpsc::channel::<()>(2);
        let (flushed_tx, mut flushed_rx) = mpsc::unbounded_channel();
        let sink = Box::pin(futures_util::sink::unfold(
            permit_rx,
            move |mut permits, _: Message| {
                let flushed = flushed_tx.clone();
                async move {
                    permits.recv().await.unwrap();
                    flushed.send(()).unwrap();
                    Ok::<_, WsClientError>(permits)
                }
            },
        ));
        let (events, _) = broadcast::channel(4);
        let (lanes, task) = spawn(sink, Duration::from_secs(60), 4, events);
        lanes.frame(data_frame(), None).unwrap();
        let (written_tx, mut written_rx) = oneshot::channel();
        lanes
            .frame_with_receipt(data_frame(), None, Some(written_tx))
            .unwrap();
        tokio::task::yield_now().await;
        assert!(matches!(
            written_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        permit_tx.send(()).await.unwrap();
        flushed_rx.recv().await.unwrap();
        let first_flushed_at = Instant::now();
        assert!(matches!(
            written_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        permit_tx.send(()).await.unwrap();
        let written_at = written_rx.await.unwrap();
        assert!(written_at >= first_flushed_at);
        drop(lanes);
        task.await.unwrap();
    }

    fn control_frame() -> PreparedClientMessage {
        PreparedClientMessage::new(&ClientMessage::Ping).expect("serialize control frame")
    }

    // Frames are queued before the writer task first runs, which holds on a
    // current-thread runtime only.
    #[tokio::test(flavor = "current_thread")]
    async fn control_lane_is_written_before_queued_data() {
        let sink = RecordingSink::default();
        let sent = Arc::clone(&sink.sent);
        let (events_tx, _) = broadcast::channel(4);
        let (lanes, task) = spawn(sink, Duration::from_secs(1), 8, events_tx);

        for _ in 0..3 {
            assert!(lanes.frame(data_frame(), None).is_ok());
        }
        assert!(lanes.frame(control_frame(), None).is_ok());
        drop(lanes);
        task.await
            .expect("writer task drains both lanes, then ends");

        let sent = sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let texts: Vec<&str> = sent
            .iter()
            .filter_map(|message| match message {
                Message::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 4);
        assert!(
            texts[0].contains("\"Ping\""),
            "control first, got {}",
            texts[0]
        );
        assert!(texts[1..].iter().all(|text| text.contains("GetMarkets")));
    }

    #[tokio::test]
    async fn stalled_socket_times_out_the_ticket_and_ends_the_writer() {
        let (events_tx, mut events_rx) = broadcast::channel(4);
        let (lanes, task) = spawn(StalledSink, Duration::from_millis(20), 8, events_tx);
        let (done_tx, done_rx) = oneshot::channel();
        assert!(lanes.frame(data_frame(), Some(done_tx)).is_ok());

        assert!(matches!(
            done_rx.await.expect("ticket resolved"),
            Err(ManagedWsError::WriteTimeout)
        ));
        task.await.expect("writer task ends after the failure");
        assert!(matches!(
            events_rx.try_recv(),
            Ok(ManagedWsEvent::Error(detail)) if detail.contains("timed out")
        ));
        assert!(matches!(
            lanes.frame(data_frame(), None),
            Err((LaneFull::Closed, None))
        ));
    }

    // Frames are queued before the writer task first runs, which holds on a
    // current-thread runtime only.
    #[tokio::test(flavor = "current_thread")]
    async fn full_lane_is_reported_without_blocking() {
        let (events_tx, _) = broadcast::channel(4);
        let (lanes, task) = spawn(StalledSink, Duration::from_secs(5), 1, events_tx);
        assert!(lanes.frame(data_frame(), None).is_ok());
        let (done_tx, mut done_rx) = oneshot::channel();
        assert!(matches!(
            lanes.frame(data_frame(), Some(done_tx)),
            Err((LaneFull::Full, Some(_)))
        ));
        assert!(
            done_rx.try_recv().is_err(),
            "the caller, not the writer, answers a rejected frame"
        );
        assert!(lanes.frame(control_frame(), None).is_ok());
        task.abort();
    }
    #[tokio::test(flavor = "current_thread")]
    async fn quote_and_cancellations_keep_fifo_order() {
        use crate::ws::types::{CancelAllQuotesMessage, CancelQuoteData, QuoteMessage};
        let quote = ClientMessage::Quote(QuoteMessage {
            rfq_id: Uuid::nil(),
            strike: 1.into(),
            price: 2.into(),
            valid_until: crate::QuoteExpiry::from_unix_seconds(100),
            nonce: 3.into(),
            order_id: crate::OrderId::new([1; 32]),
            signature: "signature".into(),
        });
        let cancel = ClientMessage::CancelQuote(CancelQuoteData {
            rfq_id: Uuid::nil(),
            request_id: Uuid::new_v4(),
        });
        let all = ClientMessage::CancelAllQuotes(CancelAllQuotesMessage {
            request_id: Uuid::new_v4(),
            market: None,
        });
        for messages in [
            [quote.clone(), cancel.clone(), all.clone()],
            [cancel.clone(), quote.clone(), all],
        ] {
            let sink = RecordingSink::default();
            let sent = Arc::clone(&sink.sent);
            let (events, _) = broadcast::channel(4);
            let (lanes, task) = spawn(sink, Duration::from_secs(1), 8, events);
            let expected: Vec<_> = messages
                .iter()
                .map(|m| serde_json::to_string(m).unwrap())
                .collect();
            for message in messages {
                assert!(
                    lanes
                        .frame(PreparedClientMessage::new(&message).unwrap(), None)
                        .is_ok()
                );
            }
            drop(lanes);
            task.await.unwrap();
            let recorded = sent.lock().unwrap();
            let actual: Vec<_> = recorded.iter().map(|m| m.to_text().unwrap()).collect();
            assert_eq!(actual, expected);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn close_does_not_drain_queued_trading_messages() {
        let sink = RecordingSink::default();
        let sent = Arc::clone(&sink.sent);
        let (events, _) = broadcast::channel(4);
        let (lanes, task) = spawn(sink, Duration::from_secs(1), 8, events);
        assert!(lanes.frame(data_frame(), None).is_ok());
        assert!(lanes.close().is_ok());
        task.await.unwrap();
        assert!(sent.lock().unwrap().is_empty());
    }
}
