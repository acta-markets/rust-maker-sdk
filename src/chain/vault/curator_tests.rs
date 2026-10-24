use super::*;
use crate::chain::vault::{
    VaultStagedSlot, VaultSwapKind,
    router_tests::{manager, quote, run},
};
use crate::types::{VaultLeg, VaultStagingStep};
use std::collections::VecDeque;

#[tokio::test]
async fn quote_fit_retries_only_three_times() {
    let mut budgets = Vec::new();
    let mut calls = 0;
    let plan = fit_with_quotes(
        |budget| {
            budgets.push(budget);
            std::future::ready(Ok(quote(&manager(), true, 0)))
        },
        |_| {
            calls += 1;
            if calls < 3 {
                Err(VaultChainError::InvalidInput(
                    "curator transaction does not fit",
                ))
            } else {
                run(Intent::Buy(1000), 0, None, 100)
            }
        },
    )
    .await
    .unwrap();
    assert!(matches!(plan, Plan::Single(_)));
    assert_eq!(budgets, [48, 36, 24]);
    budgets.clear();
    let result = fit_with_quotes(
        |budget| {
            budgets.push(budget);
            std::future::ready(Ok(quote(&manager(), true, 0)))
        },
        |_| {
            Err(VaultChainError::InvalidInput(
                "curator transaction does not fit",
            ))
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(budgets, [48, 36, 24, 12]);
}
#[tokio::test]
async fn quote_fit_does_not_retry_semantic_errors() {
    let mut calls = 0;
    let result = fit_with_quotes(
        |_| {
            calls += 1;
            std::future::ready(Ok(quote(&manager(), true, 0)))
        },
        |_| {
            Err(VaultChainError::InvalidInput(
                "Jupiter quote input mismatch",
            ))
        },
    )
    .await;
    assert_eq!(calls, 1);
    assert_eq!(
        result.unwrap_err().downcast_ref::<VaultChainError>(),
        Some(&VaultChainError::InvalidInput(
            "Jupiter quote input mismatch"
        ))
    );
}

#[tokio::test]
async fn main_slot_rolls_back_only_after_maturity() {
    let slot = VaultStagedSlot {
        mint: solana_sdk::pubkey::Pubkey::new_from_array([15; 32]),
        from_leg: VaultLeg::Main,
        amount: 1000,
        staged_at: 100,
    };
    for (now, leg) in [
        (199, VaultLeg::Second),
        (200, VaultLeg::Second),
        (201, VaultLeg::Main),
    ] {
        let plan = run(Intent::Buy(123), 0, Some(slot), now).unwrap();
        let tx = plan.first().instructions.last().unwrap();
        assert_eq!(
            u64::from_le_bytes(tx.data[3..11].try_into().unwrap()),
            slot.amount
        );
        let mut rpc = FakeRpc {
            plans: VecDeque::from([plan]),
            sent: vec![],
            fail_confirmation: false,
        };
        assert_eq!(
            execute(&mut rpc, Intent::Buy(123)).await.unwrap(),
            Outcome::Recovered {
                signature: Signature::default(),
            }
        );
        assert!(matches!(rpc.sent.as_slice(), [VaultSwapKind::Staging {
            step: VaultStagingStep::Unstage, leg: actual, generation: 8, ..
        }] if *actual == leg));
    }
    let mut staged = plans();
    staged.pop_back();
    staged.push_back(run(Intent::Buy(123), 0, Some(slot), 201).unwrap());
    let mut rpc = FakeRpc {
        plans: staged,
        sent: vec![],
        fail_confirmation: false,
    };
    assert_eq!(
        execute(&mut rpc, Intent::Buy(123)).await.unwrap(),
        Outcome::Recovered {
            signature: Signature::default(),
        }
    );
    assert_eq!(rpc.sent.len(), 2);
}

struct FakeRpc {
    plans: VecDeque<Plan>,
    sent: Vec<VaultSwapKind>,
    fail_confirmation: bool,
}
impl Driver for FakeRpc {
    async fn refresh(&mut self, _: Intent) -> Result<Plan, Error> {
        self.plans
            .pop_front()
            .ok_or_else(|| "refresh unavailable".into())
    }
    async fn send_confirmed(&mut self, parts: &TxParts) -> Result<Signature, Error> {
        self.sent.push(parts.kind.clone());
        if self.fail_confirmation {
            Err("confirmation unresolved".into())
        } else {
            Ok(Signature::default())
        }
    }
}
fn plans() -> VecDeque<Plan> {
    let staged = (1..45)
        .find_map(|n| match run(Intent::Buy(1000), n, None, 100) {
            Ok(p @ Plan::Staged { .. }) => Some(p),
            _ => None,
        })
        .unwrap();
    let slot = VaultStagedSlot {
        mint: solana_sdk::pubkey::Pubkey::new_from_array([15; 32]),
        from_leg: VaultLeg::Main,
        amount: 1000,
        staged_at: 100,
    };
    VecDeque::from([staged, run(Intent::Buy(1000), 0, Some(slot), 101).unwrap()])
}
#[tokio::test]
async fn confirmed_stage_refreshes_before_unstage() {
    let mut rpc = FakeRpc {
        plans: plans(),
        sent: vec![],
        fail_confirmation: false,
    };
    let outcome = execute(&mut rpc, Intent::Buy(1000)).await.unwrap();
    assert!(matches!(outcome, Outcome::Executed { .. }));
    assert_eq!(rpc.sent.len(), 2);
    assert!(matches!(
        rpc.sent[1],
        VaultSwapKind::Staging {
            step: VaultStagingStep::Unstage,
            ..
        }
    ));
    assert!(rpc.plans.is_empty());
}
#[tokio::test]
async fn uncertain_stage_stops_and_restart_only_unstages() {
    let mut rpc = FakeRpc {
        plans: plans(),
        sent: vec![],
        fail_confirmation: true,
    };
    assert!(execute(&mut rpc, Intent::Buy(1000)).await.is_err());
    assert_eq!(rpc.sent.len(), 1);
    assert_eq!(rpc.plans.len(), 1);
    rpc.fail_confirmation = false;
    rpc.sent.clear();
    assert!(matches!(
        execute(&mut rpc, Intent::Buy(1000)).await.unwrap(),
        Outcome::Recovered { .. }
    ));
    assert_eq!(rpc.sent.len(), 1);
    assert!(matches!(
        rpc.sent[0],
        VaultSwapKind::Staging {
            step: VaultStagingStep::Unstage,
            ..
        }
    ));
}

#[tokio::test]
async fn sell_slot_recovery_ignores_new_buy_intent() {
    let slot = VaultStagedSlot {
        mint: solana_sdk::pubkey::Pubkey::new_from_array([15; 32]),
        from_leg: VaultLeg::Second,
        amount: 1000,
        staged_at: 100,
    };
    let plan = run(Intent::Buy(123), 0, Some(slot), 101).unwrap();
    let mut rpc = FakeRpc {
        plans: VecDeque::from([plan]),
        sent: vec![],
        fail_confirmation: false,
    };
    assert_eq!(
        execute(&mut rpc, Intent::Buy(123)).await.unwrap(),
        Outcome::Recovered {
            signature: Signature::default()
        }
    );
    assert!(matches!(
        rpc.sent.as_slice(),
        [VaultSwapKind::Staging {
            step: VaultStagingStep::Unstage,
            leg: VaultLeg::Main,
            ..
        }]
    ));
}

#[tokio::test]
async fn failed_refresh_retains_confirmed_stage_signature() {
    let mut only_stage = plans();
    only_stage.pop_back();
    let mut rpc = FakeRpc {
        plans: only_stage,
        sent: vec![],
        fail_confirmation: false,
    };
    let error = execute(&mut rpc, Intent::Buy(1000)).await.unwrap_err();
    let stage = error
        .downcast_ref::<StageError>()
        .expect("typed confirmed stage error");
    assert_eq!(stage.signature, Signature::default());
    assert_eq!(stage.source.to_string(), "refresh unavailable");
    assert_eq!(rpc.sent.len(), 1);
}
#[test]
fn sell_quote_uses_conservative_base_input() {
    let price = crate::chain::vault::router::OraclePrice {
        mint: solana_sdk::pubkey::Pubkey::new_unique(),
        usd: 1.0,
        decimals: 9,
    };
    assert_eq!(sell_quote_amount(price, 1000, price, 50).unwrap(), 995);
}
#[test]
fn creates_only_missing_atas() {
    let m = manager();
    let setup = crate::chain::vault::served::create_ata_idempotent(
        m.args.actor,
        m.addresses.vault,
        solana_sdk::pubkey::Pubkey::new_unique(),
        crate::chain::ix::TOKEN_PROGRAM_ID,
    );
    let mut snapshot = BTreeMap::new();
    assert_eq!(
        missing_ata_setup(std::slice::from_ref(&setup), &snapshot),
        std::slice::from_ref(&setup)
    );
    snapshot.insert(setup.accounts[1].pubkey, Account::default());
    assert!(missing_ata_setup(&[setup], &snapshot).is_empty());
}

#[test]
fn quote_tables_must_match_rpc_contents() {
    use solana_sdk::message::AddressLookupTableAccount;
    let key = solana_sdk::pubkey::Pubkey::new_unique();
    let address = solana_sdk::pubkey::Pubkey::new_unique();
    let mut data = vec![0; 56];
    data[..4].copy_from_slice(&1u32.to_le_bytes());
    data[4..12].copy_from_slice(&u64::MAX.to_le_bytes());
    data.extend_from_slice(address.as_ref());
    let snapshot = BTreeMap::from([(
        key,
        Account {
            data,
            owner: solana_sdk::pubkey!("AddressLookupTab1e1111111111111111111111111"),
            ..Account::default()
        },
    )]);
    let mut q = quote(&manager(), true, 0);
    q.tables = vec![AddressLookupTableAccount {
        key,
        addresses: vec![address],
    }];
    verify_quote_tables(&mut q, &snapshot).unwrap();
    assert_eq!(q.tables[0].addresses, [address]);
    q.tables[0].addresses[0] = solana_sdk::pubkey::Pubkey::new_unique();
    assert_eq!(
        verify_quote_tables(&mut q, &snapshot)
            .unwrap_err()
            .to_string(),
        "Jupiter lookup table contents differ from RPC"
    );
    assert!(verify_quote_tables(&mut q, &BTreeMap::new()).is_err());
}
