use super::*;
use crate::chain::ix::{FundPositionIxArgs, build_fund_position_ixs};
use solana_sdk::{
    account::Account, hash::Hash, instruction::AccountMeta, signature::Keypair,
    transaction::Transaction,
};

fn global_config_account(program: Pubkey, bump: u8, hot_authority: Pubkey) -> Account {
    let mut data = vec![0; 296];
    data[..4].copy_from_slice(&[6, 1, bump, 0]);
    data[8..40].copy_from_slice(hot_authority.as_ref());
    Account {
        lamports: 1,
        data,
        owner: program,
        executable: false,
        rent_epoch: 0,
    }
}

#[tokio::test]
async fn hot_authority_requires_the_canonical_checked_global_config() {
    let program = Pubkey::new_unique();
    let keeper = Pubkey::new_unique();
    let (global, bump) = Pubkey::find_program_address(&[b"global_config"], &program);
    let client = ChainClient::with_accounts(
        program,
        [(global, global_config_account(program, bump, keeper))],
    );
    assert_eq!(client.hot_authority().await.unwrap(), keeper);

    let bad_bump = ChainClient::with_accounts(
        program,
        [(
            global,
            global_config_account(program, bump.wrapping_add(1), keeper),
        )],
    );
    assert!(matches!(
        bad_bump.hot_authority().await,
        Err(ChainError::Identity("global config PDA"))
    ));

    let mut wrong_owner = global_config_account(program, bump, keeper);
    wrong_owner.owner = Pubkey::new_unique();
    let wrong_owner = ChainClient::with_accounts(program, [(global, wrong_owner)]);
    assert!(matches!(
        wrong_owner.hot_authority().await,
        Err(ChainError::AccountOwnerMismatch { .. })
    ));
}

#[test]
fn transaction_signing_returns_error_for_missing_signer() {
    let fee_payer = Keypair::new();
    let missing_signer = Pubkey::new_unique();
    let instruction = Instruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![AccountMeta::new_readonly(missing_signer, true)],
        data: Vec::new(),
    };
    let mut transaction = Transaction::new_with_payer(&[instruction], Some(&fee_payer.pubkey()));
    let signers: [&(dyn Signer + Sync); 1] = [&fee_payer];

    assert!(matches!(
        sign_transaction(&mut transaction, &signers, Hash::new_unique()),
        Err(ChainError::Signing(_))
    ));
}

#[test]
fn transaction_signing_verifies_transaction() {
    let fee_payer = Keypair::new();
    let mut transaction = Transaction::new_with_payer(&[], Some(&fee_payer.pubkey()));
    let signers: [&(dyn Signer + Sync); 1] = [&fee_payer];

    sign_transaction(&mut transaction, &signers, Hash::new_unique()).unwrap();

    assert!(transaction.verify().is_ok());
}

#[tokio::test]
async fn native_funding_batches_only_the_required_accounts() {
    use crate::chain::wsol::{NATIVE_MINT, TOKEN_PROGRAM_ID};
    use serde_json::{Value, json};
    use solana_rpc_client::{
        api::{client_error, request::RpcRequest},
        rpc_client::RpcClientConfig,
        rpc_sender::{RpcSender, RpcTransportStats},
    };
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    type Replies = Arc<Mutex<VecDeque<(RpcRequest, Value, Value)>>>;
    struct Sender(Replies);
    #[async_trait::async_trait]
    impl RpcSender for Sender {
        async fn send(&self, request: RpcRequest, params: Value) -> client_error::Result<Value> {
            let (expected, expected_params, response) =
                self.0.lock().unwrap().pop_front().expect("unexpected RPC");
            assert_eq!(request, expected);
            assert_eq!(params, expected_params);
            Ok(response)
        }
        fn get_transport_stats(&self) -> RpcTransportStats {
            RpcTransportStats::default()
        }
        fn url(&self) -> String {
            "mock://funding".into()
        }
    }
    fn account(owner: Pubkey, data: Vec<u8>) -> Value {
        json!({"owner": owner.to_string(), "lamports": 10_000_000, "executable": false,
            "rentEpoch": 0, "data": [bs58::encode(data).into_string(), "base58"]})
    }
    fn response(value: Value) -> Value {
        json!({"context": {"slot": 10}, "value": value})
    }

    let program = Pubkey::new_unique();
    let maker = Pubkey::new_unique();
    let position = Pubkey::new_unique();
    let market = Pubkey::new_unique();
    let underlying = Pubkey::new_unique();
    let mut pos_data = vec![0; 424];
    pos_data[0..2].copy_from_slice(&[1, 2]);
    pos_data[4] = 1;
    pos_data[40..72].copy_from_slice(maker.as_ref());
    pos_data[72..104].copy_from_slice(market.as_ref());
    pos_data[104..112].copy_from_slice(&1_000_000_000u64.to_le_bytes());
    pos_data[112..120].copy_from_slice(&100u64.to_le_bytes());
    let mut market_data = vec![0; 416];
    market_data[0..5].copy_from_slice(&[4, 1, 0, 9, 9]);
    market_data[32..64].copy_from_slice(underlying.as_ref());
    market_data[64..96].copy_from_slice(NATIVE_MINT.as_ref());
    market_data[96..128].copy_from_slice(TOKEN_PROGRAM_ID.as_ref());
    market_data[128..160].copy_from_slice(TOKEN_PROGRAM_ID.as_ref());
    let funding_ata =
        crate::chain::ix::derive_associated_token_address(&maker, &NATIVE_MINT, &TOKEN_PROGRAM_ID);
    let position_ata = crate::chain::ix::derive_associated_token_address(
        &position,
        &NATIVE_MINT,
        &TOKEN_PROGRAM_ID,
    );
    let config = json!({"encoding": "base64+zstd", "commitment": "confirmed", "dataSlice": null, "minContextSlot": null});
    for (create_atas, self_paid, existing) in [
        (true, true, None),
        (true, true, Some(100)),
        (true, false, None),
        (false, true, None),
        (false, true, Some(30)),
    ] {
        let payer = if self_paid {
            maker
        } else {
            Pubkey::new_unique()
        };
        let mut replies = VecDeque::from([
            (
                RpcRequest::GetAccountInfo,
                json!([position.to_string(), config]),
                response(account(program, pos_data.clone())),
            ),
            (
                RpcRequest::GetAccountInfo,
                json!([market.to_string(), config]),
                response(account(program, market_data.clone())),
            ),
        ]);
        let source = existing.map_or(Value::Null, |amount: u64| {
            let mut data = vec![0; 165];
            data[..32].copy_from_slice(NATIVE_MINT.as_ref());
            data[32..64].copy_from_slice(maker.as_ref());
            data[64..72].copy_from_slice(&amount.to_le_bytes());
            account(TOKEN_PROGRAM_ID, data)
        });
        if create_atas && self_paid {
            replies.push_back((
                RpcRequest::GetMultipleAccounts,
                json!([[position_ata.to_string(), funding_ata.to_string()], config]),
                response(json!([null, source])),
            ));
            replies.push_back((
                RpcRequest::GetMinimumBalanceForRentExemption,
                json!([165]),
                json!(20),
            ));
        } else {
            replies.push_back((
                RpcRequest::GetAccountInfo,
                json!([funding_ata.to_string(), config]),
                response(source),
            ));
        }
        if existing.is_none() && create_atas && self_paid {
            replies.push_back((
                RpcRequest::GetMinimumBalanceForRentExemption,
                json!([165]),
                json!(20),
            ));
        }
        let missing = existing.is_none() && !create_atas;
        let wrapping = !missing && existing.unwrap_or(0) < 100;
        if wrapping {
            replies.push_back((
                RpcRequest::GetBalance,
                json!([maker.to_string(), {"commitment": "confirmed"}]),
                response(json!(10_000)),
            ));
        }
        let replies = Arc::new(Mutex::new(replies));
        let commitment = CommitmentConfig::confirmed();
        let client = ChainClient {
            rpc: RpcClient::new_sender(
                Sender(Arc::clone(&replies)),
                RpcClientConfig::with_commitment(commitment),
            ),
            program_id: program,
            commitment,
            #[cfg(feature = "ws-client")]
            confirmation_ws_url: None,
        };
        let result = client
            .prepare_fund_position(
                &FundPositionArgs {
                    maker_owner: maker,
                    position_pda: position,
                    create_atas,
                    native_sol: NativeSolFunding::WrapIfNeeded {
                        reserve_lamports: 10,
                    },
                },
                payer,
            )
            .await;
        assert!(
            replies.lock().unwrap().is_empty(),
            "unconsumed RPC expectations"
        );
        if missing {
            assert!(
                matches!(result, Err(ChainError::MissingFundingAccount(key)) if key == funding_ata)
            );
            continue;
        }
        let plan = result.unwrap();
        let mut expected = build_fund_position_ixs(
            &program,
            &FundPositionIxArgs {
                maker_owner: maker,
                position_pda: position,
                market_pda: market,
                position_type: 0,
                underlying_mint: underlying,
                quote_mint: NATIVE_MINT,
                underlying_token_program_id: TOKEN_PROGRAM_ID,
                quote_token_program_id: TOKEN_PROGRAM_ID,
                create_atas,
            },
            payer,
        )
        .unwrap();
        if wrapping {
            let funding = expected.pop().unwrap();
            let wrap = 100 - existing.unwrap_or(0);
            let tail = &plan.instructions[expected.len()..];
            assert_eq!(&plan.instructions[..expected.len()], &expected);
            assert_eq!(tail.len(), 3);
            assert_eq!(tail[0].accounts[1].pubkey, funding_ata);
            assert_eq!(
                tail[0].data,
                [2u32.to_le_bytes().as_slice(), wrap.to_le_bytes().as_slice()].concat()
            );
            assert_eq!(tail[1].program_id, TOKEN_PROGRAM_ID);
            assert_eq!(tail[1].accounts[0].pubkey, funding_ata);
            assert_eq!(tail[1].data, [17]);
            assert_eq!(tail[2], funding);
            let needed = wrap + 10 + if create_atas && self_paid { 40 } else { 0 };
            let budget = plan.native_budget.unwrap();
            assert!(budget.validate(needed + 5, 5).is_ok());
            assert!(matches!(
                budget.validate(needed + 4, 5),
                Err(ChainError::InsufficientNativeSol { .. })
            ));
        } else {
            assert_eq!(plan.instructions, expected);
            assert!(plan.native_budget.is_none());
        }
    }
}

#[test]
fn lookup_table_parses_active_tables_only() {
    let key = Pubkey::new_unique();
    let address = Pubkey::new_unique();
    let mut data = vec![0u8; LOOKUP_TABLE_META_LEN];
    data[..4].copy_from_slice(&1u32.to_le_bytes());
    data[4..12].copy_from_slice(&u64::MAX.to_le_bytes());
    data.extend_from_slice(address.as_ref());
    let mut account = Account {
        lamports: 1,
        data,
        owner: ADDRESS_LOOKUP_TABLE_PROGRAM_ID,
        executable: false,
        rent_epoch: 0,
    };
    let table = parse_lookup_table(key, &account).unwrap();
    assert_eq!((table.key, table.addresses), (key, vec![address]));

    account.data[4..12].copy_from_slice(&7u64.to_le_bytes());
    assert!(matches!(
        parse_lookup_table(key, &account),
        Err(ChainError::LookupTableDeactivated(found)) if found == key
    ));
    account.data[4..12].copy_from_slice(&u64::MAX.to_le_bytes());
    account.data.push(0);
    assert!(matches!(
        parse_lookup_table(key, &account),
        Err(ChainError::InvalidAccountData)
    ));
    account.owner = Pubkey::new_unique();
    assert!(matches!(
        parse_lookup_table(key, &account),
        Err(ChainError::AccountOwnerMismatch { .. })
    ));
}

#[tokio::test]
async fn confirming_conveniences_surface_on_chain_failure() {
    use serde_json::json;
    use solana_rpc_client::api::{client_error::ErrorKind, request::RpcRequest};
    let owner = solana_sdk::signature::Keypair::new();
    for deposit in [false, true] {
        let (client, calls) = ChainClient::with_submission_status(json!({
            "slot":1,"confirmations":1,"err":{"InstructionError":[0,{"Custom":42}]},
            "status":{"Err":{"InstructionError":[0,{"Custom":42}]}},"confirmationStatus":"confirmed"
        }));
        let result = if deposit {
            client
                .deposit_premium(
                    DepositPremiumArgs {
                        maker_owner: owner.pubkey(),
                        amount: 1,
                        premium_mint: Pubkey::new_unique(),
                        token_program: Some(TOKEN_PROGRAM_ID),
                        create_atas: false,
                    },
                    &owner,
                    None,
                )
                .await
        } else {
            client
                .send_instructions_with_options(vec![], &owner, &[], SendOptions::default())
                .await
        };
        assert!(
            matches!(result, Err(ChainError::Rpc(error)) if matches!(error.kind(), ErrorKind::TransactionError(_)))
        );
        assert!(
            calls
                .lock()
                .unwrap()
                .contains(&RpcRequest::GetSignatureStatuses)
        );
    }
}

#[tokio::test]
async fn confirming_call_rejects_expired_unobserved_transaction() {
    let (client, _) = ChainClient::with_submission_status(serde_json::Value::Null);
    let owner = solana_sdk::signature::Keypair::new();
    assert!(
        client
            .send_instructions_with_options(vec![], &owner, &[], SendOptions::default())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn shared_blockhash_submission_does_not_confirm_or_fetch_blockhash() {
    use solana_rpc_client::api::request::RpcRequest;
    let (client, calls) = ChainClient::with_submission_status(serde_json::Value::Null);
    let owner = solana_sdk::signature::Keypair::new();
    client
        .send_instructions_with_blockhash(
            vec![],
            &owner,
            &[],
            SendOptions::default(),
            Hash::new_unique(),
        )
        .await
        .unwrap();
    assert_eq!(*calls.lock().unwrap(), vec![RpcRequest::SendTransaction]);
}

#[cfg(feature = "ws-client")]
#[tokio::test]
async fn configured_ws_confirmation_propagates_status_fallback_failure() {
    use serde_json::json;
    let (client, _) = ChainClient::with_submission_status(json!({
        "slot":1,"confirmations":1,"err":{"InstructionError":[0,{"Custom":42}]},
        "status":{"Err":{"InstructionError":[0,{"Custom":42}]}},"confirmationStatus":"confirmed"
    }));
    let client = client.with_confirmation_ws_url("invalid://endpoint");
    let owner = solana_sdk::signature::Keypair::new();
    let error = client
        .send_instructions_with_options(vec![], &owner, &[], SendOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(error, ChainError::Confirmation(message) if message.contains("failed")));
}
