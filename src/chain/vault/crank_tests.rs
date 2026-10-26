use std::time::{Duration, Instant};

use solana_sdk::pubkey::Pubkey;

use super::{
    CapitalStep, VaultPhase, VaultSafety, VaultView, WindowClock, WindowPolicy, WindowTracker,
    WithdrawalPlan, next_step, plan_withdrawals,
};
use VaultPhase::{Active, Idle, ProcessingCapital, Settling};
use VaultSafety::{EmergencyUnwind, Frozen, Normal, WithdrawOnly};

fn view(phase: VaultPhase, safety: VaultSafety) -> VaultView {
    VaultView {
        phase,
        safety,
        equity_computed: false,
        accepting_deposits: false,
        pending_deposits: 0,
        pending_withdrawals: 0,
        staging: Default::default(),
    }
}

const POLICY: WindowPolicy = WindowPolicy {
    reopen: true,
    deposit_window: Duration::from_secs(180),
    trading_window: Duration::from_secs(600),
};
const NO_REOPEN: WindowPolicy = WindowPolicy {
    reopen: false,
    ..POLICY
};
const FRESH: WindowClock = WindowClock {
    open_for: None,
    closed_for: None,
};

fn open_for(secs: u64) -> WindowClock {
    WindowClock {
        open_for: Some(Duration::from_secs(secs)),
        closed_for: None,
    }
}

fn closed_for(secs: u64) -> WindowClock {
    WindowClock {
        open_for: None,
        closed_for: Some(Duration::from_secs(secs)),
    }
}

fn key(seed: u8) -> Pubkey {
    Pubkey::new_from_array([seed; 32])
}

#[test]
fn processing_capital_walks_equity_withdrawals_finalize_even_when_frozen() {
    for safety in [Normal, Frozen] {
        let mut v = view(ProcessingCapital, safety);
        v.pending_withdrawals = 2;
        assert_eq!(
            next_step(v, FRESH, POLICY),
            Some(CapitalStep::ComputeEquity),
            "{safety:?}"
        );
        v.equity_computed = true;
        assert_eq!(
            next_step(v, FRESH, POLICY),
            Some(CapitalStep::ProcessWithdrawals),
            "{safety:?}"
        );
        v.pending_withdrawals = 0;
        assert_eq!(
            next_step(v, FRESH, POLICY),
            Some(CapitalStep::FinalizeCycle),
            "{safety:?}"
        );
    }
}

#[test]
fn idle_pending_closes_the_window_then_prices_then_drains() {
    let mut v = view(Idle, Normal);
    v.accepting_deposits = true;
    v.pending_deposits = 3;
    assert_eq!(
        next_step(v, open_for(1), POLICY),
        Some(CapitalStep::CloseDeposits)
    );
    v.accepting_deposits = false;
    assert_eq!(
        next_step(v, closed_for(1), POLICY),
        Some(CapitalStep::ComputeEquity)
    );
    v.equity_computed = true;
    v.pending_withdrawals = 1;
    assert_eq!(
        next_step(v, closed_for(1), POLICY),
        Some(CapitalStep::ProcessDeposits)
    );
    v.pending_deposits = 0;
    assert_eq!(
        next_step(v, closed_for(1), POLICY),
        Some(CapitalStep::ProcessWithdrawals)
    );
}

#[test]
fn idle_drained_window_alternates_on_the_configured_cadence() {
    let closed = view(Idle, Normal);
    assert_eq!(next_step(closed, closed_for(599), POLICY), None);
    assert_eq!(
        next_step(closed, closed_for(600), POLICY),
        Some(CapitalStep::OpenDeposits)
    );
    assert_eq!(next_step(closed, closed_for(600), NO_REOPEN), None);
    assert_eq!(next_step(closed, FRESH, POLICY), None);
    let mut open = closed;
    open.accepting_deposits = true;
    assert_eq!(next_step(open, open_for(179), POLICY), None);
    assert_eq!(
        next_step(open, open_for(180), POLICY),
        Some(CapitalStep::CloseDeposits)
    );
}

#[test]
fn window_clock_restarts_when_the_observed_state_flips() {
    let mut state = WindowTracker::default();
    let t0 = Instant::now();
    assert_eq!(state.observe(false, t0), closed_for(0));
    let later = t0 + Duration::from_secs(700);
    assert_eq!(state.observe(false, later), closed_for(700));
    assert_eq!(state.observe(true, later), open_for(0));
    let much_later = later + Duration::from_secs(200);
    assert_eq!(state.observe(true, much_later), open_for(200));
}

#[test]
fn withdraw_only_prioritizes_payouts_then_retries_isolated_refunds() {
    let mut v = view(Idle, WithdrawOnly);
    v.pending_withdrawals = 1;
    v.pending_deposits = 1;
    assert_eq!(
        next_step(v, FRESH, POLICY),
        Some(CapitalStep::ProcessWithdrawals)
    );
    v.pending_withdrawals = 0;
    assert_eq!(
        next_step(v, FRESH, POLICY),
        Some(CapitalStep::ProcessDeposits)
    );
    v.pending_deposits = 0;
    assert_eq!(next_step(v, FRESH, POLICY), None);
}

#[test]
fn emergency_active_and_settling_are_left_alone() {
    for (phase, safety) in [
        (Idle, EmergencyUnwind),
        (ProcessingCapital, EmergencyUnwind),
        (ProcessingCapital, WithdrawOnly),
        (Active, Normal),
        (Settling, Normal),
    ] {
        let mut v = view(phase, safety);
        v.pending_withdrawals = 1;
        assert_eq!(
            next_step(v, closed_for(1000), POLICY),
            None,
            "{phase:?}/{safety:?}"
        );
    }
}

#[test]
fn frozen_idle_prices_and_processes_withdrawals_without_deposits_or_reopening() {
    let mut v = view(Idle, Frozen);
    v.pending_withdrawals = 1;
    v.pending_deposits = 1;
    assert_eq!(
        next_step(v, FRESH, POLICY),
        Some(CapitalStep::ProcessDeposits)
    );
    v.pending_deposits = 0;
    assert_eq!(
        next_step(v, FRESH, POLICY),
        Some(CapitalStep::ComputeEquity)
    );
    v.equity_computed = true;
    assert_eq!(
        next_step(v, FRESH, POLICY),
        Some(CapitalStep::ProcessWithdrawals)
    );
    v.pending_withdrawals = 0;
    assert_eq!(next_step(v, closed_for(1000), POLICY), None);
    v.pending_deposits = 0;
    v.equity_computed = false;
    assert_eq!(next_step(v, closed_for(1000), POLICY), None);
}

#[test]
fn sole_holder_exiting_takes_the_final_redemption_path() {
    let plan = plan_withdrawals(&[(key(1), 100)], 100);
    assert_eq!(plan.final_user, Some(key(1)));
    assert!(plan.partial.is_empty());
}

#[test]
fn draining_queue_holds_back_the_largest_holder() {
    let plan = plan_withdrawals(&[(key(1), 60), (key(2), 40)], 100);
    assert_eq!(plan.partial, vec![key(2)]);
    assert_eq!(plan.final_user, None);
    let plan = plan_withdrawals(&[(key(3), 20), (key(1), 50), (key(2), 30)], 100);
    assert_eq!(plan.partial, vec![key(2), key(3)]);
}

#[test]
fn partial_exits_are_batched_without_holdback() {
    let plan = plan_withdrawals(&[(key(1), 10), (key(2), 20)], 100);
    assert_eq!(plan.partial, vec![key(1), key(2)]);
    assert_eq!(plan.final_user, None);
    assert_eq!(plan_withdrawals(&[], 100), WithdrawalPlan::default());
}

#[test]
fn mixed_cycle_resolves_deposits_before_supply_draining_withdrawal() {
    let mut v = view(ProcessingCapital, Normal);
    v.pending_deposits = 1;
    v.pending_withdrawals = 1;
    assert_eq!(
        next_step(v, FRESH, POLICY),
        Some(CapitalStep::ComputeEquity)
    );
    v.equity_computed = true;
    assert_eq!(
        next_step(v, FRESH, POLICY),
        Some(CapitalStep::ProcessDeposits)
    );
    assert_eq!(
        next_step(v, FRESH, NO_REOPEN),
        Some(CapitalStep::ProcessDeposits)
    );
    v.pending_deposits = 0;
    assert_eq!(
        next_step(v, FRESH, POLICY),
        Some(CapitalStep::ProcessWithdrawals)
    );
    v.pending_withdrawals = 0;
    assert_eq!(
        next_step(v, FRESH, POLICY),
        Some(CapitalStep::FinalizeCycle)
    );
}

#[test]
fn frozen_deposits_refund_before_other_capital_steps() {
    for phase in [Idle, Active, Settling, ProcessingCapital] {
        for computed in [false, true] {
            for withdrawals in [0, 1] {
                let mut v = view(phase, Frozen);
                v.pending_deposits = 1;
                v.pending_withdrawals = withdrawals;
                v.equity_computed = computed;
                assert_eq!(
                    next_step(v, FRESH, POLICY),
                    Some(CapitalStep::ProcessDeposits)
                );
                v.pending_deposits = 0;
                if matches!(phase, Active | Settling) {
                    assert_eq!(next_step(v, FRESH, POLICY), None);
                }
            }
        }
    }
}

#[test]
fn listing_strings_parse_into_the_view_enums() {
    assert_eq!("processing_capital".parse(), Ok(ProcessingCapital));
    assert_eq!("withdraw_only".parse(), Ok(WithdrawOnly));
    assert_eq!("closing".parse(), Ok(VaultSafety::Closing));
    assert!("unknown".parse::<VaultPhase>().is_err());
    assert_eq!("emergency_unwind".parse(), Ok(EmergencyUnwind));
}

#[test]
fn staging_blocks_equity_and_finalization_but_preserves_other_steps() {
    for phase in [Idle, ProcessingCapital] {
        for safety in [Normal, Frozen] {
            let mut v = view(phase, safety);
            v.pending_withdrawals = 1;
            v.staging.open = Some(super::VaultStagedSlot {
                mint: key(1),
                from_leg: crate::types::VaultLeg::Main,
                amount: 1,
                staged_at: 1,
            });
            assert_eq!(next_step(v, FRESH, POLICY), None);
            v.equity_computed = true;
            assert_eq!(
                next_step(v, FRESH, POLICY),
                Some(CapitalStep::ProcessWithdrawals)
            );
            v.pending_withdrawals = 0;
            assert_eq!(next_step(v, FRESH, POLICY), None);
            v.pending_deposits = 1;
            assert_eq!(
                next_step(v, FRESH, POLICY),
                Some(CapitalStep::ProcessDeposits)
            );
            v.equity_computed = false;
            if safety == Normal && phase == Idle {
                v.accepting_deposits = true;
                assert_eq!(
                    next_step(v, FRESH, POLICY),
                    Some(CapitalStep::CloseDeposits)
                );
            }
        }
    }
}
