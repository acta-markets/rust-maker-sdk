use super::*;
use crate::chain::ix::{TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID};
use crate::chain::{MarketInfo, PositionInfo, PositionStatus};
use crate::{Decimals, OrderId, PositionType, Quantity, Strike, VaultDelegatePermissions};
use serde_json::{Value, json};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn manager(token2022: bool) -> VaultManager {
    VaultManager::new(VaultManagerArgs {
        program_id: key(9),
        vault_id: [8; 32],
        actor: key(1),
        delegate_authority: key(7),
        main_mint: key(6),
        second_mint: key(5),
        share_mint: key(4),
        main_token_program: if token2022 {
            TOKEN_2022_PROGRAM_ID
        } else {
            TOKEN_PROGRAM_ID
        },
        second_token_program: TOKEN_PROGRAM_ID,
        second_leg_generation: 0,
    })
    .unwrap()
}
fn compare(rows: &[Value], name: &str, ix: Instruction) {
    let expected = rows.iter().find(|r| r["name"] == name).unwrap();
    let actual = json!({"name":name, "programId":ix.program_id.to_string(), "accounts":ix.accounts.iter().map(|a| json!({"pubkey":a.pubkey.to_string(),"isSigner":a.is_signer,"isWritable":a.is_writable})).collect::<Vec<_>>(), "data":hex::encode(ix.data)});
    assert_eq!(&actual, expected, "{name}");
}
#[test]
fn ts_owner_maker_golden() {
    let rows: Vec<Value> =
        serde_json::from_str(include_str!("../../../tests/fixtures/owner_maker.json")).unwrap();
    let mut count = 0;
    for (main2022, second2022) in [(false, false), (false, true), (true, false), (true, true)] {
        let m = VaultManager::new(VaultManagerArgs {
            second_token_program: if second2022 {
                TOKEN_2022_PROGRAM_ID
            } else {
                TOKEN_PROGRAM_ID
            },
            ..manager(main2022).args
        })
        .unwrap();
        let prefix = format!(
            "{}/{}/",
            if main2022 { "2022" } else { "spl" },
            if second2022 { "2022" } else { "spl" }
        );
        let prices: Vec<_> = (20..24)
            .map(|n| AccountMeta::new_readonly(key(n), false))
            .collect();
        let mut check = |name: &str, ix| {
            compare(&rows, &format!("{prefix}{name}"), ix);
            count += 1;
        };
        check("bootstrap", m.bootstrap(key(2), 100_000_000).unwrap());
        check("top_up", m.top_up_manager_stake(123, &prices).unwrap());
        check("unlock", m.unlock_manager_stake(key(2), 456).unwrap());
        for (i, ix) in m
            .unlock_manager_stake_sequence(key(2), 456)
            .unwrap()
            .into_iter()
            .enumerate()
        {
            check(&format!("unlock_sequence/{i}"), ix);
        }
        check("accept", m.accept_governance());
        check("revoke", m.revoke_governance_nomination(key(2)).unwrap());
        check(
            "recipient",
            m.apply_fee_recipient_update(key(2), key(3)).unwrap(),
        );
        check("start", m.start_shutdown());
        check("finalize_empty", m.finalize_shutdown(&[]));
        check("finalize_dust", m.finalize_shutdown(&prices));
        check("close", m.close_vault());
        for (i, ix) in m.close_vault_sequence().into_iter().enumerate() {
            check(&format!("close_sequence/{i}"), ix);
        }
        check(
            "rotate",
            m.rotate_delegate(key(3), VaultDelegatePermissions::from_bits(3), 1000)
                .unwrap(),
        );
        check("disable", m.disable_delegate());
        for (name, update) in [
            ("capLimit", VaultGovernanceUpdate::CapLimit(123)),
            (
                "fees",
                VaultGovernanceUpdate::Fees {
                    performance_bps: 100,
                    management_bps: 20,
                },
            ),
            (
                "hwmReset",
                VaultGovernanceUpdate::HwmReset((1u128 << 80) + 123),
            ),
            (
                "capabilities",
                VaultGovernanceUpdate::Capabilities { value: 3, mask: 7 },
            ),
            (
                "policy",
                VaultGovernanceUpdate::Policy(VaultPolicyUpdate {
                    max_open_positions: 10,
                    max_rebalance_drift_bps: 100,
                    min_main_liquidity_bps: 500,
                }),
            ),
            (
                "governanceAuthority",
                VaultGovernanceUpdate::GovernanceAuthority(key(3)),
            ),
            ("secondLeg", VaultGovernanceUpdate::SecondLeg(key(3))),
        ] {
            check(
                &format!("propose/{name}"),
                m.propose_update(update).unwrap(),
            );
            check(
                &format!("cancel/{name}"),
                m.cancel_update(update.kind(), key(2)),
            );
            if update.kind() == VaultGovernanceUpdateKind::SecondLeg {
                check(
                    "apply/secondLeg_empty",
                    m.apply_second_leg(key(2), key(3), None).unwrap(),
                );
                check(
                    "apply/secondLeg_dust",
                    m.apply_second_leg(key(2), key(3), Some(&prices)).unwrap(),
                );
            } else {
                check(
                    &format!("apply/{name}"),
                    m.apply_update(update.kind(), key(2), Some(&prices))
                        .unwrap(),
                );
            }
        }
    }
    for put in [false, true] {
        for vault in [false, true] {
            for outcome in ["open_otm", "funded_otm", "funded_itm"] {
                let (mut position, mut market) = settlement(put, vault);
                position.status = if outcome == "open_otm" {
                    PositionStatus::Open
                } else {
                    PositionStatus::Funded
                };
                market.settlement_price = if outcome == "funded_itm" {
                    if put { 99 } else { 101 }
                } else {
                    100
                };
                compare(
                    &rows,
                    &format!("settle/{put}/{vault}/{outcome}"),
                    build_settle_position_ix(key(9), key(1), &position, &market).unwrap(),
                );
                count += 1;
                for (i, ix) in settle_position_sequence(key(9), key(1), &position, &market)
                    .unwrap()
                    .into_iter()
                    .enumerate()
                {
                    compare(
                        &rows,
                        &format!("settle_sequence/{put}/{vault}/{outcome}/{i}"),
                        ix,
                    );
                    count += 1;
                }
            }
        }
    }
    assert_eq!(rows.len(), count);
}
fn settlement(put: bool, vault: bool) -> (PositionInfo, MarketInfo) {
    let market = MarketInfo {
        expiry_ts: 100,
        is_put: put,
        is_finalized: true,
        settlement_price: 100,
        underlying_mint: key(6),
        quote_mint: key(5),
        underlying_token_program_id: TOKEN_PROGRAM_ID,
        quote_token_program_id: TOKEN_2022_PROGRAM_ID,
        underlying_decimals: Decimals::new(9),
        quote_decimals: Decimals::new(6),
    };
    let position = PositionInfo {
        position_type: if put {
            PositionType::CashSecuredPut
        } else {
            PositionType::CoveredCall
        },
        flags: u8::from(vault),
        flags2: 0,
        flags3: 0,
        status: PositionStatus::Open,
        taker_owner: key(2),
        maker_owner: key(1),
        market_pda: crate::quote::market_pda(&key(9), &key(6), &key(5), 100, put),
        strike: Strike::new(100),
        quantity: Quantity::new(1),
        order_id: OrderId::new([8; 32]),
    };
    (position, market)
}
#[test]
fn settlement_rejects_wrong_owner_market_and_lifecycle() {
    let (mut p, mut m) = settlement(false, false);
    assert!(build_settle_position_ix(key(9), key(2), &p, &m).is_err());
    p.flags = 2;
    assert!(build_settle_position_ix(key(9), key(1), &p, &m).is_err());
    p.flags = 0;
    m.is_finalized = false;
    assert!(build_settle_position_ix(key(9), key(1), &p, &m).is_err());
    m.is_finalized = true;
    m.settlement_price = 101;
    assert!(build_settle_position_ix(key(9), key(1), &p, &m).is_err());
    p.status = PositionStatus::Funded;
    assert!(build_settle_position_ix(key(9), key(1), &p, &m).is_ok());
    p.market_pda = key(30);
    assert!(build_settle_position_ix(key(9), key(1), &p, &m).is_err());
}
#[test]
fn lifecycle_rejects_invalid_intent() {
    let m = manager(false);
    assert!(m.bootstrap(key(2), 99_999_999).is_err());
    assert!(m.bootstrap(Pubkey::default(), 100_000_000).is_err());
    assert!(m.top_up_manager_stake(0, &[]).is_err());
    assert_eq!(
        m.top_up_manager_stake(1, &[]),
        Err(VaultChainError::MissingPriceSection)
    );
    assert!(m.unlock_manager_stake(key(2), 0).is_err());
    assert!(
        m.unlock_manager_stake(m.addresses.manager_stake_authority, 1)
            .is_err()
    );
    assert!(m.revoke_governance_nomination(Pubkey::default()).is_err());
    assert!(m.apply_fee_recipient_update(key(2), key(2)).is_err());
}

#[test]
fn general_position_decoder_preserves_unknown_flags_but_settlement_rejects_them() {
    let (p, m) = settlement(false, false);
    let mut data = vec![0; 424];
    data[0] = 1;
    data[1] = 2;
    data[4] = 1;
    data[8..40].copy_from_slice(p.taker_owner.as_ref());
    data[40..72].copy_from_slice(p.maker_owner.as_ref());
    data[72..104].copy_from_slice(p.market_pda.as_ref());
    data[104..112].copy_from_slice(&100u64.to_le_bytes());
    data[128..160].copy_from_slice(p.order_id.as_bytes());
    for (index, value) in [(5, 4), (6, 1), (7, 1)] {
        let mut bytes = data.clone();
        bytes[index] = value;
        let account = solana_sdk::account::Account {
            data: bytes,
            owner: key(9),
            lamports: 1,
            executable: false,
            rent_epoch: 0,
        };
        let decoded = PositionInfo::from_account(&account, &key(9)).unwrap();
        assert_eq!(
            [decoded.flags, decoded.flags2, decoded.flags3][index - 5],
            value
        );
        assert!(matches!(
            build_settle_position_ix(key(9), key(1), &decoded, &m),
            Err(crate::chain::ChainError::InvalidAccountData)
        ));
        assert!(settle_position_sequence(key(9), key(1), &decoded, &m).is_err());
    }
}

#[cfg(feature = "chain-rpc")]
mod rpc {
    use super::*;
    use crate::chain::ChainClient;
    use solana_sdk::account::Account;
    fn account(owner: Pubkey, data: Vec<u8>) -> Account {
        Account {
            owner,
            data,
            lamports: 1,
            executable: false,
            rent_epoch: 0,
        }
    }
    fn fixtures(
        m: &VaultManager,
        amount: u64,
        kind: VaultGovernanceUpdateKind,
    ) -> Vec<(Pubkey, Account)> {
        let mut vault = vec![0; VaultAccount::LEN];
        vault[0] = 9;
        vault[1] = 1;
        for (offset, key) in [
            (16, key(8)),
            (48, key(1)),
            (80, key(3)),
            (112, key(2)),
            (144, key(7)),
            (176, key(6)),
            (208, m.args.second_mint),
            (240, key(4)),
        ] {
            vault[offset..offset + 32].copy_from_slice(key.as_ref());
        }
        let mut token = vec![0; 165];
        token[..32].copy_from_slice(m.args.second_mint.as_ref());
        token[32..64].copy_from_slice(m.addresses.vault.as_ref());
        token[64..72].copy_from_slice(&amount.to_le_bytes());
        token[108] = 1;
        let mut proposal = vec![0; 368];
        proposal[0] = 14;
        proposal[1] = 1;
        proposal[2] = pda::pending_vault_update(&key(9), &m.addresses.vault, kind as u8).1;
        proposal[3] = kind as u8;
        proposal[8..40].copy_from_slice(m.addresses.vault.as_ref());
        proposal[40..72].copy_from_slice(key(2).as_ref());
        proposal[80..112].copy_from_slice(key(3).as_ref());
        vec![
            (m.addresses.vault, account(key(9), vault)),
            (
                m.addresses.vault_second_ata,
                account(TOKEN_PROGRAM_ID, token),
            ),
            (m.pending_update(kind), account(key(9), proposal)),
        ]
    }
    fn route(mint: Pubkey, seed: u8) -> Account {
        let mut data = vec![0; PriceRoute::LEN];
        data[0] = 15;
        data[1] = 1;
        data[3] = 1;
        data[4..6].copy_from_slice(&41u16.to_le_bytes());
        data[8..40].copy_from_slice(mint.as_ref());
        data[49..81].fill(seed);
        data[81..85].copy_from_slice(&60u32.to_le_bytes());
        account(key(9), data)
    }
    #[tokio::test]
    async fn empty_second_needs_no_routes_but_dust_does() {
        let m = manager(false);
        for kind in [
            VaultGovernanceUpdateKind::SecondLeg,
            VaultGovernanceUpdateKind::Fees,
            VaultGovernanceUpdateKind::HwmReset,
        ] {
            for amount in [0, 1] {
                let base = fixtures(&m, amount, kind);
                let chain = ChainClient::with_accounts(key(9), base.clone());
                assert_eq!(
                    chain
                        .vault_manager(m.addresses.vault, key(1))
                        .await
                        .unwrap(),
                    m
                );
                assert_eq!(
                    m.finalize_shutdown_from_chain(&chain, 7).await.is_ok(),
                    amount == 0
                );
                assert_eq!(
                    m.apply_update_from_chain(&chain, kind, 7).await.is_ok(),
                    amount == 0 && kind == VaultGovernanceUpdateKind::SecondLeg
                );
                let main = route(key(6), 20);
                let second = route(key(5), 21);
                let prices = m
                    .price_section(
                        &PriceRoute::from_account(&main, &key(9)).unwrap(),
                        &PriceRoute::from_account(&second, &key(9)).unwrap(),
                        7,
                    )
                    .unwrap();
                let mut all = base;
                all.extend([
                    (m.addresses.main_price_route, main),
                    (m.addresses.second_price_route, second),
                ]);
                let chain = ChainClient::with_accounts(key(9), all);
                let actual = m.apply_update_from_chain(&chain, kind, 7).await.unwrap();
                let expected = if kind == VaultGovernanceUpdateKind::SecondLeg {
                    m.apply_second_leg(
                        key(2),
                        key(3),
                        if amount == 0 { None } else { Some(&prices) },
                    )
                    .unwrap()
                } else {
                    m.apply_update(kind, key(2), Some(&prices)).unwrap()
                };
                assert_eq!(actual, expected);
                assert_eq!(
                    m.finalize_shutdown_from_chain(&chain, 7).await.unwrap(),
                    m.finalize_shutdown(if amount == 0 { &[] } else { &prices })
                );
                assert_eq!(
                    m.top_up_manager_stake_from_chain(&chain, 123, 7)
                        .await
                        .unwrap(),
                    m.top_up_manager_stake(123, &prices).unwrap()
                );
            }
        }
    }
    #[tokio::test]
    async fn resolves_recipients_payer_and_rejects_stale_or_foreign_state() {
        let m = manager(false);
        let kind = VaultGovernanceUpdateKind::CapLimit;
        let base = fixtures(&m, 0, kind);
        let chain = ChainClient::with_accounts(key(9), base.clone());
        assert_eq!(
            m.cancel_update_from_chain(&chain, kind).await.unwrap(),
            m.cancel_update(kind, key(2))
        );
        assert_eq!(
            m.unlock_manager_stake_from_chain(&chain, 10).await.unwrap(),
            m.unlock_manager_stake(key(2), 10).unwrap()
        );
        assert_eq!(
            m.apply_fee_recipient_update_from_chain(&chain, key(3))
                .await
                .unwrap(),
            m.apply_fee_recipient_update(key(2), key(3)).unwrap()
        );
        assert_eq!(
            m.revoke_governance_nomination_from_chain(&chain)
                .await
                .unwrap(),
            m.revoke_governance_nomination(key(3)).unwrap()
        );
        for (index, offset) in [(0, 144), (0, 208), (2, 8), (2, 3), (2, 2)] {
            let mut invalid = base.clone();
            invalid[index].1.data[offset] ^= 1;
            let chain = ChainClient::with_accounts(key(9), invalid);
            assert!(m.apply_update_from_chain(&chain, kind, 0).await.is_err());
        }
        let mut invalid = base;
        invalid[2].1.owner = key(30);
        let chain = ChainClient::with_accounts(key(9), invalid);
        assert!(m.cancel_update_from_chain(&chain, kind).await.is_err());
    }
    #[tokio::test]
    async fn native_second_uses_balance_after_sync_and_checks_reserve_encoding() {
        let m = VaultManager::new(VaultManagerArgs {
            second_mint: crate::chain::NATIVE_MINT,
            ..manager(false).args
        })
        .unwrap();
        let kind = VaultGovernanceUpdateKind::SecondLeg;
        for excess in [0, 1] {
            let mut base = fixtures(&m, 0, kind);
            base[1].1.data[109..113].copy_from_slice(&1u32.to_le_bytes());
            base[1].1.data[113..121].copy_from_slice(&100u64.to_le_bytes());
            base[1].1.lamports = 100 + excess;
            let chain = ChainClient::with_accounts(key(9), base.clone());
            assert_eq!(
                m.finalize_shutdown_from_chain(&chain, 0).await.is_ok(),
                excess == 0
            );
            assert_eq!(
                m.apply_update_from_chain(&chain, kind, 0).await.is_ok(),
                excess == 0
            );
            let main = route(key(6), 20);
            let second = route(m.args.second_mint, 21);
            let prices = m
                .price_section(
                    &PriceRoute::from_account(&main, &key(9)).unwrap(),
                    &PriceRoute::from_account(&second, &key(9)).unwrap(),
                    0,
                )
                .unwrap();
            let mut all = base.clone();
            all.extend([
                (m.addresses.main_price_route, main),
                (m.addresses.second_price_route, second),
            ]);
            let chain = ChainClient::with_accounts(key(9), all);
            assert_eq!(
                m.finalize_shutdown_from_chain(&chain, 0).await.unwrap(),
                m.finalize_shutdown(if excess == 0 { &[] } else { &prices })
            );
            assert_eq!(
                m.apply_update_from_chain(&chain, kind, 0).await.unwrap(),
                m.apply_second_leg(
                    key(2),
                    key(3),
                    if excess == 0 { None } else { Some(&prices) }
                )
                .unwrap()
            );
            for (tag, lamports, stored) in [(2u32, 100u64, 0u64), (1, 99, 0), (1, 100, 1)] {
                let mut invalid = base.clone();
                invalid[1].1.data[109..113].copy_from_slice(&tag.to_le_bytes());
                invalid[1].1.data[64..72].copy_from_slice(&stored.to_le_bytes());
                invalid[1].1.lamports = lamports;
                let chain = ChainClient::with_accounts(key(9), invalid);
                assert!(m.finalize_shutdown_from_chain(&chain, 0).await.is_err());
            }
        }
        let mut invalid = fixtures(&m, 0, kind);
        invalid[2].1.data[40..72].fill(0);
        let chain = ChainClient::with_accounts(key(9), invalid);
        assert!(m.cancel_update_from_chain(&chain, kind).await.is_err());
    }
    #[tokio::test]
    async fn owner_and_staging_guards_run_before_dependent_reads() {
        use crate::chain::ChainError;
        let m = manager(false);
        let kind = VaultGovernanceUpdateKind::CapLimit;
        let base = fixtures(&m, 0, kind);
        let mut staged = base.clone();
        staged[0].1.data[456..488].copy_from_slice(key(3).as_ref());
        staged[0].1.data[488..496].copy_from_slice(&1u64.to_le_bytes());
        let chain = ChainClient::with_accounts(key(9), staged);
        for result in [
            m.top_up_manager_stake_from_chain(&chain, 1, 0).await,
            m.finalize_shutdown_from_chain(&chain, 0).await,
            m.start_shutdown_from_chain(&chain).await,
            m.close_vault_from_chain(&chain).await,
            m.apply_update_from_chain(&chain, VaultGovernanceUpdateKind::Fees, 0)
                .await,
            m.apply_update_from_chain(&chain, VaultGovernanceUpdateKind::HwmReset, 0)
                .await,
            m.apply_update_from_chain(&chain, VaultGovernanceUpdateKind::SecondLeg, 0)
                .await,
        ] {
            assert!(matches!(
                result,
                Err(ChainError::Vault(VaultChainError::Staged))
            ));
        }
        assert!(m.unlock_manager_stake_from_chain(&chain, 1).await.is_ok());
        assert!(
            m.apply_fee_recipient_update_from_chain(&chain, key(3))
                .await
                .is_ok()
        );
        assert!(m.apply_update_from_chain(&chain, kind, 0).await.is_ok());
        let mut foreign = base.clone();
        foreign[0].1.data[48..80].copy_from_slice(key(30).as_ref());
        let chain = ChainClient::with_accounts(key(9), foreign);
        for result in [
            m.top_up_manager_stake_from_chain(&chain, 1, 0).await,
            m.unlock_manager_stake_from_chain(&chain, 1).await,
            m.apply_fee_recipient_update_from_chain(&chain, key(3))
                .await,
            m.revoke_governance_nomination_from_chain(&chain).await,
            m.start_shutdown_from_chain(&chain).await,
            m.close_vault_from_chain(&chain).await,
        ] {
            assert!(matches!(
                result,
                Err(ChainError::Vault(
                    VaultChainError::GovernanceAuthorityRequired
                ))
            ));
        }
        assert!(matches!(
            m.unlock_manager_stake_sequence_from_chain(&chain, 1).await,
            Err(ChainError::Vault(
                VaultChainError::GovernanceAuthorityRequired
            ))
        ));
        assert!(m.finalize_shutdown_from_chain(&chain, 0).await.is_ok());
        assert!(m.apply_update_from_chain(&chain, kind, 0).await.is_ok());
        let empty = ChainClient::with_accounts(key(9), []);
        assert!(matches!(
            m.cancel_update_from_chain(&empty, VaultGovernanceUpdateKind::HwmReset)
                .await,
            Err(ChainError::Vault(VaultChainError::ColdAuthorityRequired))
        ));
        let chain = ChainClient::with_accounts(key(9), base);
        assert_eq!(
            m.start_shutdown_from_chain(&chain).await.unwrap(),
            m.start_shutdown()
        );
        assert_eq!(
            m.close_vault_from_chain(&chain).await.unwrap(),
            m.close_vault()
        );
        assert_eq!(
            m.unlock_manager_stake_sequence_from_chain(&chain, 456)
                .await
                .unwrap(),
            m.unlock_manager_stake_sequence(key(2), 456).unwrap()
        );
    }

    #[tokio::test]
    async fn priced_apply_is_permissionless_even_with_new_owner_guards() {
        let m = manager(false);
        for kind in [
            VaultGovernanceUpdateKind::Fees,
            VaultGovernanceUpdateKind::HwmReset,
            VaultGovernanceUpdateKind::SecondLeg,
        ] {
            let mut base = fixtures(&m, 1, kind);
            base[0].1.data[48..80].copy_from_slice(key(30).as_ref());
            base.extend([
                (m.addresses.main_price_route, route(key(6), 20)),
                (m.addresses.second_price_route, route(key(5), 21)),
            ]);
            let chain = ChainClient::with_accounts(key(9), base);
            assert!(m.apply_update_from_chain(&chain, kind, 0).await.is_ok());
            assert!(m.finalize_shutdown_from_chain(&chain, 0).await.is_ok());
        }
    }

    #[tokio::test]
    async fn settlement_rpc_sequence_uses_decoded_snapshots() {
        let (mut p, market) = settlement(false, false);
        p.status = PositionStatus::Funded;
        let mut data = vec![0; 424];
        data[0] = 1;
        data[1] = 2;
        data[4] = 2;
        data[8..40].copy_from_slice(p.taker_owner.as_ref());
        data[40..72].copy_from_slice(p.maker_owner.as_ref());
        data[72..104].copy_from_slice(p.market_pda.as_ref());
        data[104..112].copy_from_slice(&100u64.to_le_bytes());
        data[128..160].copy_from_slice(p.order_id.as_bytes());
        let mut market_data = vec![0; 416];
        market_data[0] = 4;
        market_data[1] = 1;
        market_data[5] = 2;
        market_data[8..16].copy_from_slice(&100u64.to_le_bytes());
        market_data[16..24].copy_from_slice(&100u64.to_le_bytes());
        for (offset, key) in [
            (32, key(6)),
            (64, key(5)),
            (96, TOKEN_PROGRAM_ID),
            (128, TOKEN_2022_PROGRAM_ID),
        ] {
            market_data[offset..offset + 32].copy_from_slice(key.as_ref());
        }
        let address = pda::position(&key(9), p.order_id.as_bytes()).0;
        let chain = ChainClient::with_accounts(
            key(9),
            [
                (address, account(key(9), data)),
                (p.market_pda, account(key(9), market_data)),
            ],
        );
        assert_eq!(
            chain
                .settle_position_sequence(key(1), address)
                .await
                .unwrap(),
            settle_position_sequence(key(9), key(1), &p, &market).unwrap()
        );
    }
}
