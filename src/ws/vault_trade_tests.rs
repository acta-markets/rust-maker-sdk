use crate::wire::encode_base58;
use std::str::FromStr;

use solana_account::Account;
use solana_message::AddressLookupTableAccount;
use solana_pubkey::Pubkey;

use super::*;
use crate::chain::vault::spread_tests::{compile_with_tables, served_sale};
use crate::chain::vault::{
    ComputeBudgetBounds, Counterparty, VaultAccount, VaultIntentError, VaultManager,
    VaultManagerArgs, VaultOpenIntent, VaultOpenTerms, VaultSpreadTerms, VaultTradeTerms,
    verify_vault_open,
};
use crate::chain::{ChainClient, ChainError, MarketInfo, maker_pda_with_program_id};
use crate::spreads::{SpreadError, SpreadType};
use crate::types::{
    Nonce, OptionTerms, Price, Quantity, QuoteExpiry, Strike, TakerSide, TimeoutSeconds,
};
use crate::ws::types::{
    Delivery, QuoteReceivedMessage, ServerError, SponsoredTxToSignData,
    SubmitSignedSponsoredTxData, SwapSubmitData,
};
use crate::ws::vault::{
    SendGate, SignedTrade, TradeDesk, TradeError, TradeOutcome, TradeStage, Ungated, VaultTrade,
};

const BOUNDS: ComputeBudgetBounds = ComputeBudgetBounds {
    max_unit_limit: 400_000,
    max_unit_price_micro_lamports: 1_000,
};
const SERVED_HEIGHT: u64 = 4_242;

fn sale_case() -> serde_json::Value {
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/vault_served_tx.json")).unwrap();
    cases.into_iter().next().unwrap()
}

fn key(value: &serde_json::Value) -> Pubkey {
    Pubkey::from_str(value.as_str().unwrap()).unwrap()
}

fn num(value: &serde_json::Value) -> u64 {
    value.as_u64().unwrap()
}

fn writer(case: &serde_json::Value) -> VaultManager {
    manager(case, "writer")
}

fn manager(case: &serde_json::Value, field: &str) -> VaultManager {
    let vault = &case[field];
    VaultManager::new(VaultManagerArgs {
        program_id: key(&case["program_id"]),
        vault_id: hex::decode(vault["vault_id"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap(),
        actor: key(&vault["actor"]),
        delegate_authority: key(&vault["delegate_authority"]),
        main_mint: key(&vault["main_mint"]),
        second_mint: key(&vault["second_mint"]),
        share_mint: key(&vault["share_mint"]),
        main_token_program: key(&vault["main_token_program"]),
        second_token_program: key(&vault["second_token_program"]),
        second_leg_generation: 0,
    })
    .unwrap()
}

fn account(owner: Pubkey, data: Vec<u8>) -> Account {
    Account {
        lamports: 1,
        data,
        owner,
        executable: false,
        rent_epoch: 0,
    }
}

fn maker_account(program: Pubkey, owner: Pubkey, quote_signer: Pubkey) -> Account {
    let mut data = vec![0; 328];
    data[..2].copy_from_slice(&[2, 1]);
    data[4..36].copy_from_slice(owner.as_ref());
    data[36..68].copy_from_slice(quote_signer.as_ref());
    account(program, data)
}

fn global_config_account(program: Pubkey, hot_authority: Pubkey) -> (Pubkey, Account) {
    let (address, bump) = Pubkey::find_program_address(&[b"global_config"], &program);
    let mut data = vec![0; 296];
    data[..4].copy_from_slice(&[6, 1, bump, 0]);
    data[8..40].copy_from_slice(hot_authority.as_ref());
    (address, account(program, data))
}

fn fixture_table(case: &serde_json::Value) -> AddressLookupTableAccount {
    let table = &case["served"]["lookup_table"];
    AddressLookupTableAccount {
        key: key(&table["key"]),
        addresses: table["addresses"]
            .as_array()
            .unwrap()
            .iter()
            .map(key)
            .collect(),
    }
}

/// The fixture's chain: its maker account, checked GlobalConfig, and lookup table.
fn chain(case: &serde_json::Value) -> ChainClient {
    let program = key(&case["program_id"]);
    let owner = key(&case["maker_owner"]);
    let table = fixture_table(case);
    ChainClient::with_accounts(
        program,
        [
            (
                maker_pda_with_program_id(&program, &owner).0,
                maker_account(program, owner, key(&case["position"]["quote_signer"])),
            ),
            (
                table.key,
                ChainClient::lookup_table_account(&table.addresses),
            ),
            global_config_account(program, key(&case["keeper"])),
        ],
    )
}

fn sale(case: &serde_json::Value, window: Duration) -> VaultTrade {
    let p = &case["position"];
    VaultTrade {
        side: TakerSide::Sell,
        market_pda: key(&p["market"]),
        market: MarketInfo {
            settlement_price: 0,
            expiry_ts: num(&p["expiry_ts"]),
            is_put: p["is_put"].as_bool().unwrap(),
            is_finalized: false,
            underlying_mint: key(&p["underlying_mint"]),
            quote_mint: key(&p["quote_mint"]),
            underlying_token_program_id: Pubkey::default(),
            quote_token_program_id: Pubkey::default(),
            underlying_decimals: crate::types::Decimals::new(9),
            quote_decimals: crate::types::Decimals::new(6),
        },
        terms: OptionTerms::CoveredCall {
            strike: Strike::new(num(&p["strike"])),
        },
        quantity: Quantity::new(num(&p["quantity"])),
        rfq_timeout: TimeoutSeconds::new(20),
        window,
        bounds: BOUNDS,
    }
}

fn quote(case: &serde_json::Value, rfq_id: Uuid, price: u64, side: TakerSide) -> ServerMessage {
    let p = &case["position"];
    ServerMessage::QuoteReceived(QuoteReceivedMessage {
        rfq_id,
        terms: OptionTerms::CoveredCall {
            strike: Strike::new(num(&p["strike"])),
        },
        side,
        maker_owner: crate::Address32::from_base58(case["maker_owner"].as_str().unwrap())
            .expect("valid fixture"),
        price: Price::new(price),
        valid_until: QuoteExpiry::from_unix_seconds(num(&p["valid_until"])),
        nonce: Nonce::new(num(&p["nonce"])),
        order_id: OrderId::from_hex(p["order_id"].as_str().unwrap()).unwrap(),
        net_price: Price::new(price),
    })
}

#[derive(Debug, thiserror::Error)]
#[error("desk")]
struct DeskError;

struct Desk {
    vault: VaultManager,
    min_price: u64,
    events: Vec<&'static str>,
    /// The hook that never returns.
    stall: Option<&'static str>,
}

impl Desk {
    async fn hook(&mut self, name: &'static str) {
        self.events.push(name);
        if self.stall == Some(name) {
            std::future::pending::<()>().await;
        }
    }
}

impl TradeDesk for Desk {
    type Error = DeskError;

    async fn take(&mut self, quote: &QuoteReceivedMessage) -> Result<bool, DeskError> {
        self.hook("take").await;
        Ok(quote.price.value() >= self.min_price)
    }

    fn accepting(&mut self, _quote: &QuoteReceivedMessage) -> Result<(), DeskError> {
        self.events.push("accepting");
        Ok(())
    }

    async fn vault(&mut self) -> Result<VaultManager, DeskError> {
        self.hook("vault").await;
        Ok(self.vault)
    }

    fn signed(&mut self, sale: &SignedTrade) -> Result<(), DeskError> {
        assert_eq!(sale.last_valid_block_height, SERVED_HEIGHT);
        self.events.push("signed");
        Ok(())
    }
}

struct Closed;

impl SendGate for Closed {
    fn admit<R>(&self, _connection_epoch: u64, _enqueue: impl FnOnce() -> R) -> Option<R> {
        None
    }
}

fn spawn_writer(url: String) -> VaultOperatorClient {
    let mut config = config(url);
    config.operator = Arc::new(BytesSigner::from_secret([10; 32]));
    VaultOperatorClient::spawn(config).expect("spawn vault client")
}

async fn accept_command(socket: &mut Socket, request_id: Uuid) {
    send_server(
        socket,
        ServerMessage::Reply(crate::ws::types::ReplyMessage::accepted(request_id)),
    )
    .await;
}

#[derive(Clone, Copy)]
enum Ending {
    Accepted,
    Rejected(Delivery),
    /// The submission is never answered.
    Silent,
    /// The accepted quote is never served.
    Unserved,
    /// Served, but no submission is expected.
    Served,
    /// The server fails the accepted order and reopens the RFQ.
    Failed,
    /// The RFQ reopens without an order update.
    Released,
    /// The RFQ closes after the accept.
    Closed,
}

/// Serves one sale: RFQ, a foreign and a rejected-price quote, the taken quote,
/// the served transaction and the submission, ending as `ending` says.
async fn serve_sale(
    socket: &mut Socket,
    case: &serde_json::Value,
    ending: Ending,
) -> Option<SubmitSignedSponsoredTxData> {
    let ClientMessage::RfqRequest(rfq) = receive_client(socket).await else {
        panic!("expected RfqRequest");
    };
    assert_eq!(
        rfq.market_pda.as_str(),
        case["position"]["market"].as_str().unwrap()
    );
    let rfq_id = Uuid::new_v4();
    let created = serde_json::json!({
        "type": "RfqCreated",
        "data": {"rfq_id": rfq_id, "rfq_version": 1, "terms": rfq.terms,
            "side": rfq.side, "request_id": rfq.request_id,
            "expires_at": 1, "created_at": 1, "order_options": []},
    });
    socket
        .send(Message::Text(created.to_string().into()))
        .await
        .expect("send RfqCreated");
    accept_command(socket, rfq.request_id).await;
    let price = num(&case["position"]["price"]);
    send_server(socket, quote(case, Uuid::new_v4(), price, rfq.side)).await;
    send_server(socket, quote(case, rfq_id, price - 1, rfq.side)).await;
    send_server(socket, quote(case, rfq_id, price, rfq.side)).await;
    let ClientMessage::AcceptQuote(accept) = receive_client(socket).await else {
        panic!("expected AcceptQuote");
    };
    assert_eq!(accept.rfq_id, rfq_id);
    accept_command(socket, accept.request_id).await;
    let rfq_update = |state| {
        ServerMessage::RfqUpdate(crate::ws::types::RfqUpdateMessage {
            rfq_id,
            rfq_version: crate::RfqVersion::new(3),
            state,
        })
    };
    let available = || {
        rfq_update(crate::ws::types::RfqState::Available {
            reason: crate::types::RfqAvailableAgainReason::SignatureTimeout,
            at: std::time::SystemTime::now(),
        })
    };
    match ending {
        Ending::Unserved => return None,
        Ending::Failed => {
            send_server(
                socket,
                ServerMessage::OrderUpdate(crate::ws::types::OrderUpdateMessage {
                    order_id: accept.order_id,
                    order_version: crate::types::OrderVersion::new(2),
                    state: crate::ws::types::OrderState::Failed {
                        reason: "signature timeout".into(),
                    },
                }),
            )
            .await;
            send_server(socket, available()).await;
            return None;
        }
        Ending::Released => {
            send_server(socket, available()).await;
            return None;
        }
        Ending::Closed => {
            send_server(
                socket,
                rfq_update(crate::ws::types::RfqState::Closed {
                    reason: crate::types::RfqCloseReason::Expired,
                    winner: crate::ws::types::RfqWinner::None,
                    at: std::time::SystemTime::now(),
                }),
            )
            .await;
            return None;
        }
        _ => {}
    }
    send_server(
        socket,
        ServerMessage::SponsoredTxToSign(SponsoredTxToSignData {
            order_id: accept.order_id,
            tx_base64: case["served"]["tx_base64"].as_str().unwrap().to_owned(),
            last_valid_block_height: SERVED_HEIGHT,
            signature_deadline_at: std::time::SystemTime::now() + Duration::from_secs(60),
        }),
    )
    .await;
    if matches!(ending, Ending::Served) {
        return None;
    }
    let ClientMessage::SubmitSignedSponsoredTx(submit) = receive_client(socket).await else {
        panic!("expected SubmitSignedSponsoredTx");
    };
    match ending {
        Ending::Accepted => accept_command(socket, submit.request_id).await,
        Ending::Silent
        | Ending::Unserved
        | Ending::Served
        | Ending::Failed
        | Ending::Released
        | Ending::Closed => {}
        Ending::Rejected(delivery) => {
            send_server(
                socket,
                ServerMessage::Reply(crate::ws::types::ReplyMessage::failed(
                    submit.request_id,
                    ServerError::InternalError,
                    delivery,
                )),
            )
            .await;
        }
    }
    Some(submit)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sell_rfq_takes_a_quote_signs_the_served_sale_and_reports_delivery() {
    let case = sale_case();
    let (listener, url) = bind().await;
    let peer_case = case.clone();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
        answer_reconcile(&mut socket, ALL_READS).await;
        let mut submitted = Vec::new();
        for ending in [
            Ending::Accepted,
            Ending::Rejected(Delivery::NotForwarded),
            Ending::Rejected(Delivery::Unknown),
            Ending::Silent,
            Ending::Unserved,
        ] {
            submitted.push(serve_sale(&mut socket, &peer_case, ending).await);
        }
        drain_until_closed(socket).await;
        submitted
    });

    let client = spawn_writer(url);
    let epoch = client.wait_until_ready().await.expect("ready");
    let chain = chain(&case);
    let mut outcomes = Vec::new();
    for served in [true, true, true, true, false] {
        let mut desk = Desk {
            vault: writer(&case),
            min_price: num(&case["position"]["price"]),
            events: Vec::new(),
            stall: None,
        };
        let window = Duration::from_millis(if served { 5_000 } else { 300 });
        outcomes.push(
            client
                .trade_rfq(&chain, &sale(&case, window), epoch, &Ungated, &mut desk)
                .await,
        );
        let signed: &[&str] = if served { &["vault", "signed"] } else { &[] };
        assert_eq!(
            desk.events,
            [&["take", "take", "accepting"], signed].concat()
        );
    }
    client.close().await.expect("close");
    let submitted = peer.await.expect("peer");

    let participant_signature = |submit: &Option<SubmitSignedSponsoredTxData>| {
        let bytes = STANDARD
            .decode(&submit.as_ref().unwrap().tx_base64)
            .unwrap();
        assert_eq!(
            bytes[0], 2,
            "keeper and vault delegate must be static signers"
        );
        assert!(
            bytes[1..65].iter().all(|byte| *byte == 0),
            "the keeper's transaction-id slot is still unsigned"
        );
        encode_base58(&bytes[65..129])
    };
    let Ok(TradeOutcome::Submitted(signed)) = &outcomes[0] else {
        panic!("first sale submitted: {:?}", outcomes[0]);
    };
    assert_eq!(
        signed.participant_signature,
        participant_signature(&submitted[0])
    );
    assert_eq!(signed.order_id, submitted[0].as_ref().unwrap().order_id);
    let Err(unforwarded) = &outcomes[1] else {
        panic!("second sale rejected");
    };
    assert!(unforwarded.in_flight().is_none());
    let Err(unknown) = &outcomes[2] else {
        panic!("third sale rejected");
    };
    assert_eq!(
        unknown
            .in_flight()
            .map(|sale| sale.participant_signature.clone()),
        Some(participant_signature(&submitted[2]))
    );
    let Err(silent) = &outcomes[3] else {
        panic!("unanswered submission fails");
    };
    assert_eq!(
        silent
            .in_flight()
            .map(|sale| sale.participant_signature.clone()),
        Some(participant_signature(&submitted[3])),
        "a timed-out submission may be in flight"
    );
    assert!(
        matches!(&outcomes[4], Err(error @ TradeError::Unserved) if error.in_flight().is_none())
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trade_rfq_ends_with_the_servers_reason_when_the_accepted_order_falls_through() {
    let case = sale_case();
    let (listener, url) = bind().await;
    let peer_case = case.clone();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
        answer_reconcile(&mut socket, ALL_READS).await;
        for ending in [Ending::Failed, Ending::Released, Ending::Closed] {
            serve_sale(&mut socket, &peer_case, ending).await;
        }
        drain_until_closed(socket).await;
    });

    let client = spawn_writer(url);
    let epoch = client.wait_until_ready().await.expect("ready");
    let chain = chain(&case);
    let window = Duration::from_secs(30);
    let mut outcomes = Vec::new();
    for _ in 0..3 {
        let mut desk = Desk {
            vault: writer(&case),
            min_price: num(&case["position"]["price"]),
            events: Vec::new(),
            stall: None,
        };
        let started = tokio::time::Instant::now();
        outcomes.push(
            client
                .trade_rfq(&chain, &sale(&case, window), epoch, &Ungated, &mut desk)
                .await,
        );
        assert!(started.elapsed() < window / 2, "ended promptly");
        assert_eq!(desk.events, ["take", "take", "accepting"]);
    }
    client.close().await.expect("close");
    peer.await.expect("peer");

    assert!(
        matches!(&outcomes[0], Err(TradeError::OrderFailed(reason)) if reason == "signature timeout"),
        "{:?}",
        outcomes[0]
    );
    assert!(matches!(
        &outcomes[1],
        Err(TradeError::Released(
            crate::types::RfqAvailableAgainReason::SignatureTimeout
        ))
    ));
    assert!(matches!(
        &outcomes[2],
        Err(TradeError::Closed(crate::types::RfqCloseReason::Expired))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sell_rfq_checks_the_market_honours_the_gate_and_ends_unfilled() {
    let case = sale_case();
    let (listener, url) = bind().await;
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
        answer_reconcile(&mut socket, ALL_READS).await;
        let ClientMessage::RfqRequest(rfq) = receive_client(&mut socket).await else {
            panic!("expected RfqRequest");
        };
        accept_command(&mut socket, rfq.request_id).await;
        drain_until_closed(socket).await;
    });

    let client = spawn_writer(url);
    let epoch = client.wait_until_ready().await.expect("ready");
    let chain = chain(&case);
    let mut desk = Desk {
        vault: writer(&case),
        min_price: 1,
        events: Vec::new(),
        stall: None,
    };
    let mut elsewhere = sale(&case, Duration::from_millis(100));
    elsewhere.market_pda = Pubkey::new_unique();
    assert!(matches!(
        client
            .trade_rfq(&chain, &elsewhere, epoch, &Ungated, &mut desk)
            .await,
        Err(TradeError::Market)
    ));
    let sale = sale(&case, Duration::from_millis(200));
    let refused = client
        .trade_rfq(&chain, &sale, epoch, &Closed, &mut desk)
        .await
        .expect_err("gate refuses");
    let TradeError::Rejected {
        stage: TradeStage::Request,
        error,
    } = &refused
    else {
        panic!("gate refusal: {refused:?}");
    };
    assert!(matches!(error, VaultRequestError::Gated));
    assert_eq!(error.delivery(), Some(Delivery::NotForwarded));
    assert!(matches!(
        client.trade_rfq(&chain, &sale, epoch + 1, &Ungated, &mut desk).await,
        Err(TradeError::Rejected { error, .. }) if error.delivery() == Some(Delivery::NotForwarded)
    ));
    assert_eq!(
        client
            .trade_rfq(&chain, &sale, epoch, &Ungated, &mut desk)
            .await
            .expect("unfilled"),
        TradeOutcome::Unfilled
    );
    assert!(desk.events.is_empty());
    client.close().await.expect("close");
    peer.await.expect("peer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn swap_submit_is_bound_to_its_epoch_and_gate() {
    let (listener, url) = bind().await;
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
        answer_reconcile(&mut socket, ALL_READS).await;
        let ClientMessage::SwapSubmit(submit) = receive_client(&mut socket).await else {
            panic!("expected SwapSubmit");
        };
        send_server(
            &mut socket,
            ServerMessage::Reply(crate::ws::types::ReplyMessage {
                request_id: submit.request_id,
                result: crate::ws::types::ReplyResult::SwapSubmitted {
                    execution_id: submit.execution_id,
                },
            }),
        )
        .await;
        drain_until_closed(socket).await;
    });
    let client = VaultOperatorClient::spawn(config(url)).expect("spawn");
    let epoch = client.wait_until_ready().await.expect("ready");
    let submit = || SwapSubmitData {
        request_id: Uuid::new_v4(),
        execution_id: OrderId::new([3; 32]),
        signed_tx_base64: "tx".into(),
    };
    for (gate_open, connection_epoch) in [(false, epoch), (true, epoch + 1)] {
        let refused = if gate_open {
            client
                .swap_submit_in_epoch(submit(), connection_epoch, &Ungated)
                .await
        } else {
            client
                .swap_submit_in_epoch(submit(), connection_epoch, &Closed)
                .await
        }
        .expect_err("not sent");
        assert_eq!(refused.delivery(), Some(Delivery::NotForwarded));
    }
    let receipt = client
        .swap_submit_in_epoch(submit(), epoch, &Ungated)
        .await
        .expect("receipt");
    assert_eq!(receipt, OrderId::new([3; 32]));
    client.close().await.expect("close");
    peer.await.expect("peer");
}

#[tokio::test]
async fn counterparty_resolves_a_maker_or_a_holder_vault_from_chain() {
    let case = sale_case();
    let program = key(&case["program_id"]);
    let owner = key(&case["maker_owner"]);
    let signer = key(&case["position"]["quote_signer"]);
    let Counterparty::Maker {
        owner: loaded,
        quote_signer,
    } = Counterparty::load(&chain(&case), owner).await.unwrap()
    else {
        panic!("maker counterparty");
    };
    assert_eq!((loaded, quote_signer), (owner, signer));

    let impostor = ChainClient::with_accounts(
        program,
        [(
            maker_pda_with_program_id(&program, &owner).0,
            maker_account(program, Pubkey::new_unique(), signer),
        )],
    );
    assert!(matches!(
        Counterparty::load(&impostor, owner).await,
        Err(ChainError::Identity("maker owner"))
    ));

    let holder = writer(&case);
    let fields = &case["writer"];
    let mut data = vec![0; VaultAccount::LEN];
    data[..2].copy_from_slice(&[9, 1]);
    data[16..48].copy_from_slice(&hex::decode(fields["vault_id"].as_str().unwrap()).unwrap());
    for (offset, field) in [
        (144, "delegate_authority"),
        (176, "main_mint"),
        (208, "second_mint"),
        (240, "share_mint"),
    ] {
        data[offset..offset + 32].copy_from_slice(key(&fields[field]).as_ref());
    }
    for (offset, field) in [(402, "main_token_program"), (403, "second_token_program")] {
        data[offset] = u8::from(key(&fields[field]) == crate::chain::ix::TOKEN_2022_PROGRAM_ID);
    }
    let vaults = ChainClient::with_accounts(
        program,
        [(holder.addresses.vault, account(program, data.clone()))],
    );
    let Counterparty::Vault(loaded) = Counterparty::load(&vaults, holder.addresses.vault)
        .await
        .unwrap()
    else {
        panic!("vault counterparty");
    };
    assert_eq!(loaded.addresses.vault, holder.addresses.vault);
    let elsewhere = Pubkey::new_unique();
    let moved = ChainClient::with_accounts(program, [(elsewhere, account(program, data))]);
    assert!(matches!(
        Counterparty::load(&moved, elsewhere).await,
        Err(ChainError::Identity("holder vault PDA"))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stalled_desk_times_out_before_anything_is_signed() {
    let case = sale_case();
    let (listener, url) = bind().await;
    let peer_case = case.clone();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
        answer_reconcile(&mut socket, ALL_READS).await;
        let ClientMessage::RfqRequest(rfq) = receive_client(&mut socket).await else {
            panic!("expected RfqRequest");
        };
        accept_command(&mut socket, rfq.request_id).await;
        let created = serde_json::json!({
            "type": "RfqCreated",
            "data": {"rfq_id": Uuid::from_u128(9), "rfq_version": 1, "terms": rfq.terms,
                "side": rfq.side, "request_id": rfq.request_id,
                "expires_at": 1, "created_at": 1, "order_options": []},
        });
        socket
            .send(Message::Text(created.to_string().into()))
            .await
            .expect("send RfqCreated");
        let price = num(&peer_case["position"]["price"]);
        send_server(
            &mut socket,
            quote(&peer_case, Uuid::from_u128(9), price, TakerSide::Sell),
        )
        .await;
        let _ = serve_sale(&mut socket, &peer_case, Ending::Served).await;
        drain_until_closed(socket).await;
    });

    let client = spawn_writer(url);
    let epoch = client.wait_until_ready().await.expect("ready");
    let chain = chain(&case);
    let sale = sale(&case, Duration::from_millis(300));
    for (stall, stage, events) in [
        ("take", TradeStage::Take, &["take"][..]),
        (
            "vault",
            TradeStage::Sign,
            &["take", "take", "accepting", "vault"][..],
        ),
    ] {
        let mut desk = Desk {
            vault: writer(&case),
            min_price: num(&case["position"]["price"]),
            events: Vec::new(),
            stall: Some(stall),
        };
        let error = client
            .trade_rfq(&chain, &sale, epoch, &Ungated, &mut desk)
            .await
            .expect_err("stalled desk");
        assert!(
            matches!(&error, TradeError::TimedOut(timed_out) if *timed_out == stage),
            "{error:?}"
        );
        assert!(error.in_flight().is_none());
        assert_eq!(desk.events, events);
    }
    client.close().await.expect("close");
    peer.await.expect("peer");
}

const SPREAD_QUOTE_SIGNER: [u8; 32] = [21; 32];

/// A spread on the fixture sale's pair and expiry, K2 four premiums above K1.
fn spread_sale(case: &serde_json::Value, put: bool) -> VaultTrade {
    let p = &case["position"];
    let lower_strike = Strike::new(num(&p["strike"]));
    let upper_strike = Strike::new(num(&p["strike"]) + 4 * num(&p["price"]));
    let mut sale = sale(case, Duration::from_secs(5));
    sale.market.is_put = put;
    sale.market_pda = crate::quote::market_pda(
        &key(&case["program_id"]),
        &key(&p["underlying_mint"]),
        &key(&p["quote_mint"]),
        num(&p["expiry_ts"]),
        put,
    );
    sale.terms = if put {
        OptionTerms::PutSpread {
            lower_strike,
            upper_strike,
        }
    } else {
        OptionTerms::CallSpread {
            lower_strike,
            upper_strike,
        }
    };
    sale
}

/// The fixture maker's quote for `sale` and the vault open it binds.
fn spread_quote(
    case: &serde_json::Value,
    sale: &VaultTrade,
    rfq_id: Uuid,
) -> (QuoteReceivedMessage, crate::chain::vault::VaultOpenPosition) {
    let p = &case["position"];
    let mut quote = QuoteReceivedMessage {
        rfq_id,
        terms: sale.terms,
        side: TakerSide::Sell,
        maker_owner: crate::Address32::from_base58(case["maker_owner"].as_str().unwrap())
            .expect("valid fixture"),
        price: Price::new(num(&p["price"])),
        valid_until: QuoteExpiry::from_unix_seconds(num(&p["valid_until"])),
        nonce: Nonce::new(num(&p["nonce"])),
        order_id: OrderId::new([0; 32]),
        net_price: Price::new(num(&p["price"])),
    };
    let VaultOpenTerms::Spread(terms) = sale.signed_terms(&quote).unwrap() else {
        panic!("spread sale signs spread terms");
    };
    let open = writer(case)
        .open_spread_as_writer(key(&case["keeper"]), key(&case["maker_owner"]), &terms)
        .unwrap();
    quote.order_id = OrderId::new(open.order_id);
    (quote, open)
}

#[test]
fn vault_sale_signs_both_spread_strikes_and_the_single_option_strike() {
    let case = sale_case();
    let p = &case["position"];
    for (put, kind) in [
        (false, SpreadType::CallSpread),
        (true, SpreadType::PutSpread),
    ] {
        let sale = spread_sale(&case, put);
        let (quote, _) = spread_quote(&case, &sale, Uuid::nil());
        assert_eq!(
            sale.signed_terms(&quote).unwrap(),
            VaultOpenTerms::Spread(VaultSpreadTerms {
                underlying_mint: key(&p["underlying_mint"]),
                quote_mint: key(&p["quote_mint"]),
                expiry_ts: num(&p["expiry_ts"]),
                kind,
                side: TakerSide::Sell,
                lower_strike: num(&p["strike"]),
                upper_strike: num(&p["strike"]) + 4 * num(&p["price"]),
                quantity: num(&p["quantity"]),
                price: num(&p["price"]),
                valid_until: num(&p["valid_until"]),
                nonce: num(&p["nonce"]),
            })
        );
    }
    let ServerMessage::QuoteReceived(quote) =
        quote(&case, Uuid::nil(), num(&p["price"]), TakerSide::Sell)
    else {
        panic!("quote");
    };
    assert_eq!(
        sale(&case, Duration::from_secs(1))
            .signed_terms(&quote)
            .unwrap(),
        VaultOpenTerms::Physical(VaultTradeTerms {
            underlying_mint: key(&p["underlying_mint"]),
            quote_mint: key(&p["quote_mint"]),
            expiry_ts: num(&p["expiry_ts"]),
            is_put: false,
            side: TakerSide::Sell,
            strike: num(&p["strike"]),
            quantity: num(&p["quantity"]),
            price: num(&p["price"]),
            valid_until: num(&p["valid_until"]),
            nonce: num(&p["nonce"]),
        })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sell_rfq_sells_call_and_put_spreads_and_refuses_a_tampered_one() {
    let case = sale_case();
    let signer = BytesSigner::from_secret(SPREAD_QUOTE_SIGNER);
    // Call and put are served honestly; the last call is served with K2 moved.
    let rounds: Vec<(VaultTrade, Uuid, bool)> = [(false, true), (true, true), (false, false)]
        .into_iter()
        .map(|(put, honest)| (spread_sale(&case, put), Uuid::new_v4(), honest))
        .collect();
    let served: Vec<(QuoteReceivedMessage, String, bool)> = rounds
        .iter()
        .map(|(sale, rfq_id, honest)| {
            let (quote, open) = spread_quote(&case, sale, *rfq_id);
            let keeper = key(&case["keeper"]);
            let mut ixs = served_sale(keeper, &open, &signer);
            if !honest {
                ixs[4].data[74] ^= 1;
            }
            let tx_base64 = compile_with_tables(keeper, &ixs, &[fixture_table(&case)]);
            if *honest {
                let writer = writer(&case);
                let intent = VaultOpenIntent::Sale {
                    keeper,
                    writer: &writer,
                    maker_owner: key(&case["maker_owner"]),
                    maker_quote_signer: Pubkey::new_from_array(signer.pubkey_bytes()),
                    terms: sale.signed_terms(&quote).unwrap(),
                };
                let to_sign = SponsoredTxToSignData {
                    order_id: quote.order_id,
                    tx_base64: tx_base64.clone(),
                    last_valid_block_height: SERVED_HEIGHT,
                    signature_deadline_at: std::time::SystemTime::now() + Duration::from_secs(60),
                };
                verify_vault_open(&to_sign, &intent, &[fixture_table(&case)], BOUNDS)
                    .unwrap_or_else(|error| panic!("spread served transaction rejected: {error}"));
            }
            (quote, tx_base64, *honest)
        })
        .collect();
    let (listener, url) = bind().await;
    let peer_served = served.clone();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
        answer_reconcile(&mut socket, ALL_READS).await;
        let mut submitted = Vec::new();
        for (quote, tx_base64, honest) in peer_served {
            let ClientMessage::RfqRequest(rfq) = receive_client(&mut socket).await else {
                panic!("expected RfqRequest");
            };
            assert_eq!((rfq.terms, rfq.side), (quote.terms, TakerSide::Sell));
            let created = serde_json::json!({
                "type": "RfqCreated",
                "data": {"rfq_id": quote.rfq_id, "rfq_version": 1, "terms": rfq.terms,
                    "side": rfq.side, "request_id": rfq.request_id,
                    "expires_at": 1, "created_at": 1, "order_options": []},
            });
            socket
                .send(Message::Text(created.to_string().into()))
                .await
                .expect("send RfqCreated");
            accept_command(&mut socket, rfq.request_id).await;
            send_server(&mut socket, ServerMessage::QuoteReceived(quote.clone())).await;
            let ClientMessage::AcceptQuote(accept) = receive_client(&mut socket).await else {
                panic!("expected AcceptQuote");
            };
            assert_eq!(accept.order_id, quote.order_id);
            accept_command(&mut socket, accept.request_id).await;
            send_server(
                &mut socket,
                ServerMessage::SponsoredTxToSign(SponsoredTxToSignData {
                    order_id: quote.order_id,
                    tx_base64,
                    last_valid_block_height: SERVED_HEIGHT,
                    signature_deadline_at: std::time::SystemTime::now() + Duration::from_secs(60),
                }),
            )
            .await;
            if honest {
                let ClientMessage::SubmitSignedSponsoredTx(submit) =
                    receive_client(&mut socket).await
                else {
                    panic!("expected SubmitSignedSponsoredTx");
                };
                accept_command(&mut socket, submit.request_id).await;
                submitted.push(submit);
            }
        }
        drain_until_closed(socket).await;
        submitted
    });

    let client = spawn_writer(url);
    let epoch = client.wait_until_ready().await.expect("ready");
    let program = key(&case["program_id"]);
    let owner = key(&case["maker_owner"]);
    let table = fixture_table(&case);
    let chain = ChainClient::with_accounts(
        program,
        [
            (
                maker_pda_with_program_id(&program, &owner).0,
                maker_account(
                    program,
                    owner,
                    Pubkey::new_from_array(signer.pubkey_bytes()),
                ),
            ),
            (
                table.key,
                ChainClient::lookup_table_account(&table.addresses),
            ),
            global_config_account(program, key(&case["keeper"])),
        ],
    );
    let desk = || Desk {
        vault: writer(&case),
        min_price: num(&case["position"]["price"]),
        events: Vec::new(),
        stall: None,
    };

    // Refused before anything is sent: a put product on a call market, and K1 >= K2 or K1 = 0.
    let call = &rounds[0].0;
    let OptionTerms::CallSpread {
        lower_strike,
        upper_strike,
    } = call.terms
    else {
        panic!("call spread");
    };
    let put_on_call = VaultTrade {
        terms: OptionTerms::PutSpread {
            lower_strike,
            upper_strike,
        },
        ..call.clone()
    };
    assert!(matches!(
        client
            .trade_rfq(&chain, &put_on_call, epoch, &Ungated, &mut desk())
            .await,
        Err(TradeError::Market)
    ));
    for (lower_strike, upper_strike) in [
        (upper_strike, lower_strike),
        (lower_strike, lower_strike),
        (Strike::new(0), upper_strike),
    ] {
        let invalid = VaultTrade {
            terms: OptionTerms::CallSpread {
                lower_strike,
                upper_strike,
            },
            ..call.clone()
        };
        assert!(matches!(
            client
                .trade_rfq(&chain, &invalid, epoch, &Ungated, &mut desk())
                .await,
            Err(TradeError::Spread(SpreadError::InvalidStrikes))
        ));
    }

    let mut outcomes = Vec::new();
    for (sale, _, _) in &rounds {
        let mut desk = desk();
        outcomes.push(
            client
                .trade_rfq(&chain, sale, epoch, &Ungated, &mut desk)
                .await,
        );
    }
    client.close().await.expect("close");
    let submitted = peer.await.expect("peer");
    assert_eq!(submitted.len(), 2);
    for ((outcome, submit), (quote, _, _)) in outcomes.iter().zip(&submitted).zip(&served) {
        let Ok(TradeOutcome::Submitted(signed)) = outcome else {
            panic!("spread sale submitted: {outcome:?}");
        };
        assert_eq!(
            (signed.order_id, submit.order_id),
            (quote.order_id, quote.order_id)
        );
        let bytes = STANDARD.decode(&submit.tx_base64).unwrap();
        assert_eq!(signed.participant_signature, encode_base58(&bytes[65..129]));
        assert_signed_by(&submit.tx_base64, 1, &BytesSigner::from_secret([10; 32]));
    }
    let Err(tampered) = &outcomes[2] else {
        panic!("tampered spread signed: {:?}", outcomes[2]);
    };
    assert!(
        matches!(
            tampered,
            TradeError::Sign(VaultTxError::Intent(VaultIntentError::Data {
                index: 4,
                offset: 74
            }))
        ),
        "{tampered:?}"
    );
    assert!(tampered.in_flight().is_none());
}

fn purchase_case() -> serde_json::Value {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../tests/fixtures/vault_served_taker_buy_tx.json"
    ))
    .unwrap();
    cases
        .into_iter()
        .find(|case| case["kind"] == "vault_taker_buy")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trade_rfq_buys_from_a_maker_and_signs_the_served_purchase() {
    let case = purchase_case();
    let (listener, url) = bind().await;
    let peer_case = case.clone();
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::ALL).await;
        answer_reconcile(&mut socket, ALL_READS).await;
        let submitted = serve_sale(&mut socket, &peer_case, Ending::Accepted).await;
        drain_until_closed(socket).await;
        submitted
    });

    let client = spawn_writer(url);
    let epoch = client.wait_until_ready().await.expect("ready");
    let mut desk = Desk {
        vault: manager(&case, "holder"),
        min_price: num(&case["position"]["price"]),
        events: Vec::new(),
        stall: None,
    };
    let purchase = VaultTrade {
        side: TakerSide::Buy,
        ..sale(&case, Duration::from_secs(5))
    };
    let outcome = client
        .trade_rfq(&chain(&case), &purchase, epoch, &Ungated, &mut desk)
        .await;
    client.close().await.expect("close");
    let submitted = peer.await.expect("peer").expect("submitted");

    let Ok(TradeOutcome::Submitted(signed)) = outcome else {
        panic!("purchase submitted: {outcome:?}");
    };
    assert_eq!(signed.order_id, submitted.order_id);
    assert_eq!(
        desk.events,
        ["take", "take", "accepting", "vault", "signed"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trade_rfq_refuses_a_side_the_operator_may_not_take() {
    let case = purchase_case();
    let (listener, url) = bind().await;
    let peer = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        vault_handshake(&mut socket, VaultDelegatePermissions::OPEN_AS_WRITER).await;
        answer_reconcile(&mut socket, TAKER_READS).await;
        drain_until_closed(socket).await;
    });

    let client = spawn_writer(url);
    let epoch = client.wait_until_ready().await.expect("ready");
    let mut desk = Desk {
        vault: manager(&case, "holder"),
        min_price: 0,
        events: Vec::new(),
        stall: None,
    };
    let purchase = VaultTrade {
        side: TakerSide::Buy,
        ..sale(&case, Duration::from_secs(1))
    };
    let outcome = client
        .trade_rfq(&chain(&case), &purchase, epoch, &Ungated, &mut desk)
        .await;
    assert!(matches!(
        outcome,
        Err(TradeError::Rejected {
            stage: TradeStage::Request,
            error: VaultRequestError::SideNotPermitted,
        })
    ));
    assert!(desk.events.is_empty());
    client.close().await.expect("close");
    peer.await.expect("peer");
}
