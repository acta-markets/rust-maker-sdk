use super::*;
use crate::chain::rpc::ChainClient;

fn account(owner: Pubkey, data: Vec<u8>) -> Account {
    Account {
        owner,
        data,
        lamports: 1,
        executable: false,
        rent_epoch: 0,
    }
}

fn fixtures(program: Pubkey, owner: Pubkey, market: Pubkey) -> (Account, Account) {
    let mut p = vec![0; 424];
    p[0] = 1;
    p[1] = 2;
    p[3] = 1;
    p[4] = 1;
    p[40..72].copy_from_slice(owner.as_ref());
    p[72..104].copy_from_slice(market.as_ref());
    p[112..120].copy_from_slice(&123u64.to_le_bytes());
    let mut m = vec![0; 416];
    m[0] = 4;
    m[1] = 1;
    m[5] = 1;
    m[96..128].copy_from_slice(ix::TOKEN_PROGRAM_ID.as_ref());
    m[128..160].copy_from_slice(ix::TOKEN_PROGRAM_ID.as_ref());
    (account(program, p), account(program, m))
}

#[tokio::test]
async fn snapshots_chunk_deduplicate_markets_and_preserve_missing() {
    let program = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let market = Pubkey::new_unique();
    let (p, m) = fixtures(program, owner, market);
    let keys: Vec<_> = (0..201).map(|_| Pubkey::new_unique()).collect();
    let mut accounts: Vec<_> = keys.iter().map(|key| (*key, p.clone())).collect();
    accounts.push((market, m));
    let client = ChainClient::with_accounts(program, accounts);
    assert!(client.fetch_accounts(&[]).await.unwrap().is_empty());
    let missing = Pubkey::new_unique();
    let mut requested = keys.clone();
    requested.push(missing);
    let snapshot = client
        .maker_snapshot(&owner, &requested, &[])
        .await
        .unwrap();
    assert_eq!(snapshot.positions.len(), 201);
    assert_eq!(snapshot.missing_positions, vec![missing]);
    assert_eq!(snapshot.positions[0].required_settlement().unwrap(), 123);
    let position = &snapshot.positions[0];
    let instructions = position
        .fund_instructions(&program, owner, owner, true)
        .unwrap();
    assert_eq!(instructions.len(), 3);
    assert!(
        position
            .fund_instructions(&program, Pubkey::new_unique(), owner, false)
            .is_err()
    );
    let mut mismatched = position.clone();
    mismatched.market_address = Pubkey::new_unique();
    assert!(
        mismatched
            .fund_instructions(&program, owner, owner, false)
            .is_err()
    );
    let mut funded = position.clone();
    funded.position.status = PositionStatus::Funded;
    assert!(
        funded
            .fund_instructions(&program, owner, owner, false)
            .is_err()
    );
    assert!(
        client
            .maker_snapshot(&Pubkey::new_unique(), &keys, &[])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn recovery_filters_owner_and_discriminator_and_decodes_status() {
    let program = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let market = Pubkey::new_unique();
    let (p, m) = fixtures(program, owner, market);
    let (other, _) = fixtures(program, Pubkey::new_unique(), market);
    let mut settled = p.clone();
    settled.data[4] = 4;
    let key = Pubkey::new_unique();
    let client = ChainClient::with_accounts(
        program,
        [
            (key, p),
            (market, m),
            (Pubkey::new_unique(), other),
            (Pubkey::new_unique(), settled),
        ],
    );
    let positions = client.maker_open_positions(&owner).await.unwrap();
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].0, key);
    let filters = serde_json::to_value(maker_position_filters(&owner)).unwrap();
    assert_eq!(filters[2]["memcmp"]["offset"], 40);
    assert_eq!(filters[1]["memcmp"]["offset"], 0);
}

#[tokio::test]
async fn premium_decoding_distinguishes_absence_and_invalid_data() {
    let program = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let maker = ix::maker_pda_with_program_id(&program, &owner).0;
    let token = PremiumToken {
        mint: Pubkey::new_unique(),
        token_program: ix::TOKEN_PROGRAM_ID,
    };
    let ata = ix::derive_associated_token_address(&maker, &token.mint, &token.token_program);
    let mut data = vec![0; 165];
    data[..32].copy_from_slice(token.mint.as_ref());
    data[32..64].copy_from_slice(maker.as_ref());
    data[64..72].copy_from_slice(&42u64.to_le_bytes());
    data[108] = 1;
    let mut a = account(token.token_program, data);
    assert_eq!(decode_premium_balance(&a, &maker, token).unwrap(), 42);
    assert!(decode_premium_balance(&a, &owner, token).is_err());
    a.data[108] = 2;
    assert!(decode_premium_balance(&a, &maker, token).is_err());
    a.data[108] = 1;
    let client = ChainClient::with_accounts(program, [(ata, a)]);
    assert_eq!(
        client
            .maker_snapshot(&owner, &[], &[token])
            .await
            .unwrap()
            .premium_balances[0]
            .amount,
        Some(42)
    );
    let empty = ChainClient::with_accounts(program, []);
    let unsupported = PremiumToken {
        token_program: Pubkey::new_unique(),
        ..token
    };
    assert!(
        empty
            .maker_snapshot(&owner, &[], &[unsupported])
            .await
            .is_err()
    );
    assert_eq!(
        empty
            .maker_snapshot(&owner, &[], &[token])
            .await
            .unwrap()
            .premium_balances[0]
            .amount,
        None
    );
}

#[test]
fn open_positions_can_be_funded_after_market_finalization() {
    let program = Pubkey::new_unique();
    let owner = Pubkey::new_unique();
    let market_address = Pubkey::new_unique();
    let (position, market) = fixtures(program, owner, market_address);
    let mut snapshot = PositionSnapshot {
        address: Pubkey::new_unique(),
        position: PositionInfo::from_account(&position, &program).unwrap(),
        market_address,
        market: MarketInfo::from_account(&market, &program).unwrap(),
    };
    for is_put in [false, true] {
        snapshot.market.is_put = is_put;
        snapshot.position.terms = if is_put {
            crate::OptionTerms::CashSecuredPut {
                strike: snapshot.position.terms.strike(),
            }
        } else {
            crate::OptionTerms::CoveredCall {
                strike: snapshot.position.terms.strike(),
            }
        };
        snapshot.market.is_finalized = true;
        assert_eq!(snapshot.position.status, PositionStatus::Open);
        for create_atas in [false, true] {
            let instructions = snapshot
                .fund_instructions(&program, owner, owner, create_atas)
                .unwrap();
            assert_eq!(instructions.len(), if create_atas { 3 } else { 1 });
        }
    }
}
