use std::str::FromStr;

use solana_sdk::account::Account;
use solana_sdk::pubkey::Pubkey;

use super::*;
use crate::chain::vault::{
    ComputeBudgetBounds, Counterparty, VaultAccount, VaultManager, VaultManagerArgs,
};
use crate::chain::{ChainClient, ChainError, MarketInfo, maker_pda_with_program_id};
use crate::types::{Nonce, Price, Quantity, QuoteExpiry, Strike, TimeoutSeconds};
use crate::ws::types::{
    CommandAcceptedData, Delivery, QuoteReceivedMessage, RequestErrorEnvelope, ServerError,
    SponsoredTxToSignData, SubmitSignedSponsoredTxData, SwapSubmitData, SwapSubmitResultData,
};
use crate::ws::vault::{
    SaleDesk, SaleError, SaleOutcome, SaleStage, SendGate, SignedSale, Ungated, VaultSale,
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
    let vault = &case["writer"];
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

/// The fixture's chain: its maker account and lookup table.
fn chain(case: &serde_json::Value) -> ChainClient {
    let program = key(&case["program_id"]);
    let owner = key(&case["maker_owner"]);
    let table = &case["served"]["lookup_table"];
    let addresses: Vec<Pubkey> = table["addresses"]
        .as_array()
        .unwrap()
        .iter()
        .map(key)
        .collect();
    ChainClient::with_accounts(
        program,
        [
            (
                maker_pda_with_program_id(&program, &owner).0,
                maker_account(program, owner, key(&case["position"]["quote_signer"])),
            ),
            (
                key(&table["key"]),
                ChainClient::lookup_table_account(&addresses),
            ),
        ],
    )
}

fn sale(case: &serde_json::Value, window: Duration) -> VaultSale {
    let p = &case["position"];
    VaultSale {
        market_pda: key(&p["market"]),
        market: MarketInfo {
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
        strike: Strike::new(num(&p["strike"])),
        quantity: Quantity::new(num(&p["quantity"])),
        rfq_timeout: TimeoutSeconds::new(20),
        window,
        bounds: BOUNDS,
    }
}

fn quote(case: &serde_json::Value, rfq_id: Uuid, price: u64) -> ServerMessage {
    let p = &case["position"];
    ServerMessage::QuoteReceived(QuoteReceivedMessage {
        rfq_id,
        strike: Strike::new(num(&p["strike"])),
        maker: case["maker_owner"].as_str().unwrap().to_owned(),
        price: Price::new(price),
        valid_until: QuoteExpiry::from_unix_seconds(num(&p["valid_until"])),
        nonce: Nonce::new(num(&p["nonce"])),
        order_id: OrderId::from_hex(p["order_id"].as_str().unwrap()).unwrap(),
        net_price: None,
    })
}

#[derive(Debug, thiserror::Error)]
#[error("desk")]
struct DeskError;

struct Desk {
    writer: VaultManager,
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

impl SaleDesk for Desk {
    type Error = DeskError;

    async fn take(&mut self, quote: &QuoteReceivedMessage) -> Result<bool, DeskError> {
        self.hook("take").await;
        Ok(quote.price.value() >= self.min_price)
    }

    fn accepting(&mut self, _quote: &QuoteReceivedMessage) -> Result<(), DeskError> {
        self.events.push("accepting");
        Ok(())
    }

    async fn writer(&mut self) -> Result<VaultManager, DeskError> {
        self.hook("writer").await;
        Ok(self.writer)
    }

    fn signed(&mut self, sale: &SignedSale) -> Result<(), DeskError> {
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
        ServerMessage::CommandAccepted(CommandAcceptedData { request_id }),
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
        rfq.market.as_str(),
        case["position"]["market"].as_str().unwrap()
    );
    let rfq_id = Uuid::new_v4();
    let created = serde_json::json!({
        "type": "RfqCreated",
        "data": {"rfq_id": rfq_id, "rfq_version": 1, "request_id": rfq.request_id,
            "expires_at": 1, "created_at": 1},
    });
    socket
        .send(Message::Text(created.to_string().into()))
        .await
        .expect("send RfqCreated");
    accept_command(socket, rfq.request_id).await;
    let price = num(&case["position"]["price"]);
    send_server(socket, quote(case, Uuid::new_v4(), price)).await;
    send_server(socket, quote(case, rfq_id, price - 1)).await;
    send_server(socket, quote(case, rfq_id, price)).await;
    let ClientMessage::AcceptQuote(accept) = receive_client(socket).await else {
        panic!("expected AcceptQuote");
    };
    assert_eq!(accept.rfq_id, rfq_id);
    accept_command(socket, accept.request_id).await;
    if matches!(ending, Ending::Unserved) {
        return None;
    }
    send_server(
        socket,
        ServerMessage::SponsoredTxToSign(SponsoredTxToSignData {
            order_id: accept.order_id,
            tx_base64: case["served"]["tx_base64"].as_str().unwrap().to_owned(),
            last_valid_block_height: SERVED_HEIGHT,
            signature_deadline: std::time::SystemTime::now() + Duration::from_secs(60),
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
        Ending::Silent | Ending::Unserved | Ending::Served => {}
        Ending::Rejected(delivery) => {
            send_server(
                socket,
                ServerMessage::RequestError(RequestErrorEnvelope {
                    request_id: submit.request_id,
                    error: ServerError::InternalError,
                    delivery: Some(delivery),
                }),
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
            writer: writer(&case),
            min_price: num(&case["position"]["price"]),
            events: Vec::new(),
            stall: None,
        };
        let window = Duration::from_millis(if served { 5_000 } else { 300 });
        outcomes.push(
            client
                .sell_rfq(&chain, &sale(&case, window), epoch, &Ungated, &mut desk)
                .await,
        );
        let signed: &[&str] = if served { &["writer", "signed"] } else { &[] };
        assert_eq!(
            desk.events,
            [&["take", "take", "accepting"], signed].concat()
        );
    }
    client.close().await.expect("close");
    let submitted = peer.await.expect("peer");

    let signature = |submit: &Option<SubmitSignedSponsoredTxData>| {
        let bytes = STANDARD
            .decode(&submit.as_ref().unwrap().tx_base64)
            .unwrap();
        encode_base58(&bytes[1..65])
    };
    let Ok(SaleOutcome::Submitted(signed)) = &outcomes[0] else {
        panic!("first sale submitted: {:?}", outcomes[0]);
    };
    assert_eq!(signed.signature, signature(&submitted[0]));
    assert_eq!(signed.order_id, submitted[0].as_ref().unwrap().order_id);
    let Err(unforwarded) = &outcomes[1] else {
        panic!("second sale rejected");
    };
    assert!(unforwarded.in_flight().is_none());
    let Err(unknown) = &outcomes[2] else {
        panic!("third sale rejected");
    };
    assert_eq!(
        unknown.in_flight().map(|sale| sale.signature.clone()),
        Some(signature(&submitted[2]))
    );
    let Err(silent) = &outcomes[3] else {
        panic!("unanswered submission fails");
    };
    assert_eq!(
        silent.in_flight().map(|sale| sale.signature.clone()),
        Some(signature(&submitted[3])),
        "a timed-out submission may be in flight"
    );
    assert!(
        matches!(&outcomes[4], Err(error @ SaleError::Unserved) if error.in_flight().is_none())
    );
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
        writer: writer(&case),
        min_price: 1,
        events: Vec::new(),
        stall: None,
    };
    let mut elsewhere = sale(&case, Duration::from_millis(100));
    elsewhere.market_pda = Pubkey::new_unique();
    assert!(matches!(
        client
            .sell_rfq(&chain, &elsewhere, epoch, &Ungated, &mut desk)
            .await,
        Err(SaleError::Market)
    ));
    let sale = sale(&case, Duration::from_millis(200));
    let refused = client
        .sell_rfq(&chain, &sale, epoch, &Closed, &mut desk)
        .await
        .expect_err("gate refuses");
    let SaleError::Rejected {
        stage: SaleStage::Request,
        error,
    } = &refused
    else {
        panic!("gate refusal: {refused:?}");
    };
    assert!(matches!(error, VaultRequestError::Gated));
    assert_eq!(error.delivery(), Some(Delivery::NotForwarded));
    assert!(matches!(
        client.sell_rfq(&chain, &sale, epoch + 1, &Ungated, &mut desk).await,
        Err(SaleError::Rejected { error, .. }) if error.delivery() == Some(Delivery::NotForwarded)
    ));
    assert_eq!(
        client
            .sell_rfq(&chain, &sale, epoch, &Ungated, &mut desk)
            .await
            .expect("unfilled"),
        SaleOutcome::Unfilled
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
            ServerMessage::SwapSubmitResult(SwapSubmitResultData {
                request_id: submit.request_id,
                execution_id: submit.execution_id,
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
    assert_eq!(receipt.execution_id, OrderId::new([3; 32]));
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
            "data": {"rfq_id": Uuid::from_u128(9), "rfq_version": 1, "request_id": rfq.request_id,
                "expires_at": 1, "created_at": 1},
        });
        socket
            .send(Message::Text(created.to_string().into()))
            .await
            .expect("send RfqCreated");
        let price = num(&peer_case["position"]["price"]);
        send_server(&mut socket, quote(&peer_case, Uuid::from_u128(9), price)).await;
        let _ = serve_sale(&mut socket, &peer_case, Ending::Served).await;
        drain_until_closed(socket).await;
    });

    let client = spawn_writer(url);
    let epoch = client.wait_until_ready().await.expect("ready");
    let chain = chain(&case);
    let sale = sale(&case, Duration::from_millis(300));
    for (stall, stage, events) in [
        ("take", SaleStage::Take, &["take"][..]),
        (
            "writer",
            SaleStage::Sign,
            &["take", "take", "accepting", "writer"][..],
        ),
    ] {
        let mut desk = Desk {
            writer: writer(&case),
            min_price: num(&case["position"]["price"]),
            events: Vec::new(),
            stall: Some(stall),
        };
        let error = client
            .sell_rfq(&chain, &sale, epoch, &Ungated, &mut desk)
            .await
            .expect_err("stalled desk");
        assert!(
            matches!(&error, SaleError::TimedOut(timed_out) if *timed_out == stage),
            "{error:?}"
        );
        assert!(error.in_flight().is_none());
        assert_eq!(desk.events, events);
    }
    client.close().await.expect("close");
    peer.await.expect("peer");
}
