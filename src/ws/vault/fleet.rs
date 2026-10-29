use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio::time::Instant;
use uuid::Uuid;

use std::future::Future;
use std::pin::Pin;

use crate::http::{HttpClient, HttpError, Vault};
use crate::orders::SignerLike;
use crate::ws::auth::validate_wallet_auth_challenge;
use crate::ws::client::WsClient;
use crate::ws::types::{
    AuthChallengeData, ClientMessage, HelloData, ServerMessage, StartAuthData, SubscribeData,
    WsChannel,
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const MIN_INTERVAL: Duration = Duration::from_millis(100);
const MAX_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone)]
pub struct VaultFleetConfig {
    /// API origin or any maker WebSocket URL; the feed connects to its `/taker` path.
    pub ws_url: String,
    pub hello: HelloData,
    pub operator: Arc<dyn SignerLike + Send + Sync>,
    pub api: HttpClient,
    retry: Duration,
    timeout: Duration,
}

impl VaultFleetConfig {
    #[must_use]
    pub fn new(
        ws_url: impl Into<String>,
        hello: HelloData,
        operator: Arc<dyn SignerLike + Send + Sync>,
        api: HttpClient,
    ) -> Self {
        Self {
            ws_url: ws_url.into(),
            hello,
            operator,
            api,
            retry: Duration::from_secs(5),
            timeout: Duration::from_secs(5),
        }
    }

    /// First reconnect delay of the feed and the wait between failed listings;
    /// clamped to 100 ms..=60 s.
    #[must_use]
    pub fn with_retry(mut self, retry: Duration) -> Self {
        self.retry = retry.clamp(MIN_INTERVAL, MAX_INTERVAL);
        self
    }

    /// Deadline for the feed to subscribe, and its liveness probe period once
    /// subscribed (silent for two periods ends it); clamped to 100 ms..=60 s.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout.clamp(MIN_INTERVAL, MAX_INTERVAL);
        self
    }
}

/// The vaults the operator may run, as last listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FleetListing {
    /// No listing has completed yet.
    Pending,
    /// Vault PDAs whose current delegate grant names the operator, listed
    /// while the catalog feed is live.
    Listed(Arc<BTreeSet<String>>),
    /// The latest listing failed or the catalog feed is down; the last
    /// listing is no longer known current.
    Failed(Arc<str>),
}

/// Follows the vault catalog for one operator: a `/taker` session subscribed to
/// `vaults` re-lists `GET /api/v1/vaults?delegate=` on every subscription and
/// catalog change, never on a timer. Dropping it stops the feed.
pub struct VaultFleet {
    listing: watch::Receiver<FleetListing>,
    task: tokio::task::JoinHandle<()>,
}

impl VaultFleet {
    #[must_use]
    pub fn discover(config: VaultFleetConfig) -> Self {
        let (tx, listing) = watch::channel(FleetListing::Pending);
        Self {
            listing,
            task: tokio::spawn(follow(config, tx)),
        }
    }

    /// Changes whenever a listing completes or fails, or the feed is lost.
    #[must_use]
    pub fn listings(&self) -> watch::Receiver<FleetListing> {
        self.listing.clone()
    }
}

impl Drop for VaultFleet {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn follow(config: VaultFleetConfig, tx: watch::Sender<FleetListing>) {
    let operator = config.operator.pubkey_base58();
    let mut delay = config.retry;
    loop {
        if let Err(error) = session(&config, &operator, &tx, &mut delay).await {
            tracing::warn!(%error, "Vault catalog feed failed; reconnecting");
            tx.send_replace(FleetListing::Failed(
                format!("catalog feed: {error}").into(),
            ));
        }
        tokio::time::sleep(delay).await;
        delay = crate::ws::reconnect::next_reconnect_delay(delay, MAX_INTERVAL);
    }
}

type Listing = Pin<
    Box<
        dyn Future<Output = Result<Result<Vec<Vault>, HttpError>, tokio::time::error::Elapsed>>
            + Send,
    >,
>;

/// Runs one feed session; it only ends with an error. Every write, the
/// subscription and each listing are bounded by `timeout`; a listing in flight
/// when the session ends is dropped unpublished.
async fn session(
    config: &VaultFleetConfig,
    operator: &str,
    tx: &watch::Sender<FleetListing>,
    delay: &mut Duration,
) -> Result<(), BoxError> {
    let bound = config.timeout;
    let mut silent_at = Instant::now() + bound;
    let mut ws = within(
        bound,
        "connect",
        WsClient::connect(&taker_url(&config.ws_url)),
    )
    .await?;
    within(
        bound,
        "hello",
        ws.send(&ClientMessage::Hello(config.hello.clone())),
    )
    .await?;
    within(bound, "welcome", validate_welcome(&mut ws, &config.hello)).await?;
    within(
        bound,
        "start auth",
        ws.send(&ClientMessage::StartAuth(StartAuthData {
            pubkey: crate::Address32::new(config.operator.pubkey_bytes()),
        })),
    )
    .await?;
    let subscribe_id = Uuid::new_v4();
    let mut probe_at: Option<Instant> = None;
    let mut relist_at: Option<Instant> = None;
    let mut listing: Option<Listing> = None;
    let mut wanted = false;
    loop {
        if wanted && listing.is_none() {
            let api = config.api.clone();
            let operator = crate::Address32::new(config.operator.pubkey_bytes());
            listing = Some(Box::pin(tokio::time::timeout(bound, async move {
                api.delegated_vaults(operator).await
            })));
            wanted = false;
            relist_at = None;
        }
        let message =
            match next_event(ws.next(), &mut listing, relist_at, probe_at, silent_at).await {
                Event::Socket(message) => message,
                Event::Listed(listed) => {
                    listing = None;
                    relist_at = publish(tx, listed, config.retry);
                    continue;
                }
                Event::Relist => {
                    wanted = true;
                    continue;
                }
                Event::Probe => {
                    within(bound, "ping", ws.send(&ClientMessage::Ping)).await?;
                    probe_at = Some(Instant::now() + bound);
                    continue;
                }
                Event::Silent => {
                    return Err(if probe_at.is_some() {
                        "went silent".into()
                    } else {
                        "did not subscribe in time".into()
                    });
                }
            };
        let Some(message) = message else {
            return Err("closed".into());
        };
        let message = message?;
        if probe_at.is_some() {
            silent_at = Instant::now() + bound * 2;
        }
        match message {
            ServerMessage::AuthRequest(request) => {
                validate_wallet_auth_challenge(&request.challenge, operator)?;
                let signature = config
                    .operator
                    .sign_message_base58(request.challenge.as_bytes());
                within(
                    bound,
                    "auth challenge",
                    ws.send(&ClientMessage::AuthChallenge(AuthChallengeData {
                        challenge: request.challenge,
                        signature,
                        pubkey: crate::Address32::new(config.operator.pubkey_bytes()),
                    })),
                )
                .await?;
            }
            ServerMessage::AuthSuccess(_) => {
                within(
                    bound,
                    "subscribe",
                    ws.send(&ClientMessage::Subscribe(SubscribeData {
                        products: crate::ProductSet::ALL,
                        request_id: subscribe_id,
                        channels: vec![WsChannel::Vaults],
                        underlying_mints: vec![],
                        quote_mints: vec![],
                    })),
                )
                .await?;
            }
            ServerMessage::SubscribeAck(ack) if ack.request_id == subscribe_id => {
                *delay = config.retry;
                probe_at = Some(Instant::now() + bound);
                silent_at = Instant::now() + bound * 2;
                wanted = true;
            }
            ServerMessage::VaultCatalogChanged(_) => wanted = true,
            ServerMessage::VersionMismatch(mismatch) => {
                return Err(format!("protocol version mismatch: {mismatch:?}").into());
            }
            ServerMessage::AuthError(error) => {
                return Err(format!("auth rejected: {:?}", error.reason).into());
            }
            ServerMessage::RequestError(error) if error.request_id == subscribe_id => {
                return Err(format!("subscription rejected: {:?}", error.error).into());
            }
            _ => {}
        }
    }
}

async fn validate_welcome(ws: &mut WsClient, hello: &HelloData) -> Result<(), BoxError> {
    let message = ws
        .next()
        .await
        .ok_or_else(|| "connection closed before Welcome".to_owned())??;
    match message {
        ServerMessage::Welcome(welcome) if welcome.protocol_version == hello.protocol_version => {
            Ok(())
        }
        ServerMessage::VersionMismatch(mismatch) => {
            Err(format!("protocol version mismatch: {mismatch:?}").into())
        }
        ServerMessage::AuthError(error) => Err(format!("auth rejected: {:?}", error.reason).into()),
        ServerMessage::Error(error) => Err(format!("server error: {error:?}").into()),
        other => Err(format!("expected Welcome, received {other:?}").into()),
    }
}

type Listed = Result<Result<Vec<Vault>, HttpError>, tokio::time::error::Elapsed>;

enum Event<M> {
    Socket(M),
    Listed(Listed),
    Relist,
    Probe,
    Silent,
}

/// The socket comes first: a feed that ended must win over a listing that
/// finished in the same poll, or that listing would be published for a dead feed.
async fn next_event<M>(
    socket: impl Future<Output = M>,
    listing: &mut Option<Listing>,
    relist_at: Option<Instant>,
    probe_at: Option<Instant>,
    silent_at: Instant,
) -> Event<M> {
    tokio::select! {
        biased;
        message = socket => Event::Socket(message),
        listed = in_flight(listing) => Event::Listed(listed),
        () = due_at(relist_at) => Event::Relist,
        () = due_at(probe_at) => Event::Probe,
        () = tokio::time::sleep_until(silent_at) => Event::Silent,
    }
}

async fn within<T, E: Into<BoxError>>(
    bound: Duration,
    what: &'static str,
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, BoxError> {
    match tokio::time::timeout(bound, future).await {
        Ok(result) => result.map_err(Into::into),
        Err(_) => Err(format!("{what} timed out").into()),
    }
}

async fn in_flight(listing: &mut Option<Listing>) -> Listed {
    match listing {
        Some(listing) => listing.await,
        None => std::future::pending().await,
    }
}

async fn due_at(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Publishes a finished listing; a failure schedules the next one `retry` later.
fn publish(tx: &watch::Sender<FleetListing>, listed: Listed, retry: Duration) -> Option<Instant> {
    let error = match listed {
        Ok(Ok(vaults)) => {
            let pdas = vaults.into_iter().map(|vault| vault.vault_pda).collect();
            tx.send_replace(FleetListing::Listed(Arc::new(pdas)));
            return None;
        }
        Ok(Err(error)) => error.to_string(),
        Err(_) => "listing timed out".to_owned(),
    };
    tracing::warn!(%error, "Delegated vault listing failed");
    tx.send_replace(FleetListing::Failed(error.into()));
    Some(Instant::now() + retry)
}

fn taker_url(url: &str) -> String {
    format!("{}/taker", crate::ws::managed::ws_base_url(url))
}

#[cfg(test)]
mod tests {
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio_tungstenite::WebSocketStream;
    use tokio_tungstenite::tungstenite::Message;

    use super::*;
    use crate::orders::BytesSigner;
    use crate::ws::types::{
        AuthRequestData, AuthSuccessData, SubscribeAckData, VaultCatalogChangedData, WelcomeData,
    };

    const VAULTS: &str = include_str!("../../../tests/fixtures/http_vaults.json");
    const FIRST: &str = "4vJ9JU1bJJE96FWSJKvHsmmFADCg4gpZQff4P3bkLKi";
    const SECOND: &str = "3EKkiwNLWqoUbzFkPrmKbtUB4EweE6f4STzevYUmezeL";

    type Socket = WebSocketStream<TcpStream>;

    /// Answers each listing with the next reply and reports the requested path.
    async fn api(
        replies: Vec<(&'static str, String)>,
    ) -> (String, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (paths, requested) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            for (status, body) in replies {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let read = stream.read(&mut request).await.unwrap();
                let line = String::from_utf8_lossy(&request[..read]).into_owned();
                let _ = paths.send(line.split_whitespace().nth(1).unwrap().to_owned());
                let status = match status {
                    "slow" => {
                        tokio::time::sleep(Duration::from_millis(600)).await;
                        "200 OK"
                    }
                    status => status,
                };
                let reply = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        (base, requested)
    }

    fn listed(pdas: &[&str]) -> (&'static str, String) {
        let all: serde_json::Value = serde_json::from_str(VAULTS).unwrap();
        let vaults: Vec<_> = all["vaults"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|vault| pdas.contains(&vault["vault_pda"].as_str().unwrap()))
            .cloned()
            .collect();
        (
            "200 OK",
            serde_json::json!({ "vaults": vaults }).to_string(),
        )
    }

    async fn send(socket: &mut Socket, message: ServerMessage) {
        let text = serde_json::to_string(&message).unwrap();
        socket.send(Message::Text(text.into())).await.unwrap();
    }

    async fn receive(socket: &mut Socket) -> ClientMessage {
        loop {
            if let Message::Text(text) = socket.next().await.unwrap().unwrap() {
                return serde_json::from_str(&text).unwrap();
            }
        }
    }

    async fn accept(listener: &TcpListener) -> Socket {
        let (stream, _) = listener.accept().await.unwrap();
        tokio_tungstenite::accept_async(stream).await.unwrap()
    }

    #[test]
    fn the_feed_targets_the_taker_endpoint() {
        for url in [
            "ws://h:1",
            "ws://h:1/",
            "ws://h:1/maker",
            "http://h:1/vault",
        ] {
            assert_eq!(taker_url(url), "ws://h:1/taker", "{url}");
        }
    }

    /// Hello, wallet auth, `vaults` subscription.
    async fn subscribe(socket: &mut Socket, wallet: &str) {
        assert!(matches!(receive(socket).await, ClientMessage::Hello(_)));
        assert!(
            tokio::time::timeout(Duration::from_millis(25), receive(socket))
                .await
                .is_err(),
            "fleet must validate Welcome before StartAuth"
        );
        send(
            socket,
            ServerMessage::Welcome(WelcomeData {
                protocol_version: crate::WS_PROTOCOL_VERSION.into(),
                server_version: "test".into(),
                min_supported_version: crate::WS_PROTOCOL_VERSION.into(),

                server_time_unix_ms: std::time::UNIX_EPOCH,
                min_quote_validity_seconds: 100,
                settlement_buffer_seconds: 90,
            }),
        )
        .await;
        let ClientMessage::StartAuth(start) = receive(socket).await else {
            panic!("expected StartAuth");
        };
        assert_eq!(
            start.pubkey,
            crate::Address32::from_base58(wallet).expect("valid fixture")
        );
        let challenge = crate::ws::auth::TEST_MAKER_AUTH_CHALLENGE
            .replace("Nonce: ", &format!("Wallet: {wallet}\nNonce: "));
        send(
            socket,
            ServerMessage::AuthRequest(AuthRequestData {
                challenge: challenge.clone(),
            }),
        )
        .await;
        let ClientMessage::AuthChallenge(answer) = receive(socket).await else {
            panic!("expected AuthChallenge");
        };
        assert_eq!(answer.challenge, challenge);
        send(
            socket,
            ServerMessage::AuthSuccess(AuthSuccessData {
                session_id: "s".into(),
                expires_at: std::time::UNIX_EPOCH,
                maker_pda: None,
            }),
        )
        .await;
        let ClientMessage::Subscribe(subscribe) = receive(socket).await else {
            panic!("expected Subscribe");
        };
        assert_eq!(subscribe.channels, [WsChannel::Vaults]);
        send(
            socket,
            ServerMessage::SubscribeAck(SubscribeAckData {
                request_id: subscribe.request_id,
                subscribed: vec![WsChannel::Vaults],
            }),
        )
        .await;
    }

    async fn fleet(api: String, timeout: Duration) -> (VaultFleet, TcpListener, String) {
        let operator = Arc::new(BytesSigner::from_secret([4; 32]));
        let wallet = operator.pubkey_base58();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws_url = format!("ws://{}/maker", listener.local_addr().unwrap());
        let fleet = VaultFleet::discover(
            VaultFleetConfig::new(
                ws_url,
                HelloData {
                    protocol_version: crate::WS_PROTOCOL_VERSION.into(),
                    on_disconnect: crate::ws::types::OnDisconnect::KeepQuotes,
                    client_name: "test".to_string(),
                    client_version: "test".to_string(),
                },
                operator,
                HttpClient::new(api).unwrap(),
            )
            .with_retry(Duration::ZERO)
            .with_timeout(timeout),
        );
        (fleet, listener, wallet)
    }

    async fn next(listings: &mut watch::Receiver<FleetListing>) -> FleetListing {
        listings.changed().await.unwrap();
        listings.borrow_and_update().clone()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn fleet_relists_on_subscription_and_catalog_changes() {
        let (base, mut paths) = api(vec![listed(&[FIRST]), listed(&[FIRST, SECOND])]).await;
        let (fleet, listener, wallet) = fleet(base, Duration::from_secs(5)).await;
        let mut listings = fleet.listings();
        let mut socket = accept(&listener).await;
        subscribe(&mut socket, &wallet).await;
        assert_eq!(
            next(&mut listings).await,
            FleetListing::Listed(Arc::new(BTreeSet::from([FIRST.to_owned()])))
        );
        send(
            &mut socket,
            ServerMessage::VaultCatalogChanged(VaultCatalogChangedData {
                vault_pda: crate::Address32::from_base58(SECOND).expect("fixture"),
            }),
        )
        .await;
        assert_eq!(
            next(&mut listings).await,
            FleetListing::Listed(Arc::new(BTreeSet::from([
                FIRST.to_owned(),
                SECOND.to_owned()
            ])))
        );
        let expected = format!("/api/v1/vaults?delegate={wallet}");
        for _ in 0..2 {
            assert_eq!(paths.recv().await.unwrap(), expected);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_listing_retries_while_the_feed_is_live() {
        let (base, _paths) = api(vec![
            ("500 Internal Server Error", "{}".to_owned()),
            listed(&[]),
        ])
        .await;
        let (fleet, listener, wallet) = fleet(base, Duration::from_secs(5)).await;
        let mut listings = fleet.listings();
        let mut socket = accept(&listener).await;
        subscribe(&mut socket, &wallet).await;
        assert!(matches!(next(&mut listings).await, FleetListing::Failed(_)));
        assert_eq!(
            next(&mut listings).await,
            FleetListing::Listed(Arc::default())
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_silent_feed_is_unhealthy_and_reconnects() {
        let (base, _paths) = api(vec![listed(&[FIRST])]).await;
        let (fleet, listener, wallet) = fleet(base, Duration::from_millis(100)).await;
        let mut listings = fleet.listings();
        let _never_answered = accept(&listener).await;
        let FleetListing::Failed(reason) = next(&mut listings).await else {
            panic!("a feed that never subscribes is unhealthy");
        };
        assert!(reason.contains("welcome timed out"), "{reason}");
        let mut socket = accept(&listener).await;
        subscribe(&mut socket, &wallet).await;
        assert!(matches!(next(&mut listings).await, FleetListing::Listed(_)));
        assert!(matches!(receive(&mut socket).await, ClientMessage::Ping));
        let FleetListing::Failed(reason) = next(&mut listings).await else {
            panic!("an unanswered probe makes the feed unhealthy");
        };
        assert!(reason.contains("went silent"), "{reason}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_listing_that_outlives_its_feed_is_not_published() {
        let (base, mut paths) = api(vec![("slow", listed(&[FIRST]).1)]).await;
        let (fleet, listener, wallet) = fleet(base, Duration::from_millis(400)).await;
        let mut listings = fleet.listings();
        let mut socket = accept(&listener).await;
        subscribe(&mut socket, &wallet).await;
        let _requested = paths.recv().await.unwrap();
        drop(socket);
        let FleetListing::Failed(reason) = next(&mut listings).await else {
            panic!("a closed feed is unhealthy");
        };
        assert!(reason.contains("catalog feed"), "{reason}");
        let late = tokio::time::timeout(Duration::from_millis(900), async {
            loop {
                if matches!(next(&mut listings).await, FleetListing::Listed(_)) {
                    return;
                }
            }
        })
        .await;
        assert!(late.is_err(), "the listing finished after its feed died");
    }

    #[tokio::test]
    async fn a_dead_feed_wins_over_a_listing_that_finished_with_it() {
        let silent_at = Instant::now() + Duration::from_secs(60);
        for _ in 0..64 {
            let mut listing: Option<Listing> =
                Some(Box::pin(std::future::ready(Ok(Ok(Vec::new())))));
            let event = next_event(
                std::future::ready(None::<()>),
                &mut listing,
                Some(Instant::now()),
                Some(Instant::now()),
                silent_at,
            )
            .await;
            assert!(
                matches!(event, Event::Socket(None)),
                "the closed feed must win"
            );
        }
    }
}
