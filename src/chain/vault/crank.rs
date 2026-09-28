use std::time::{Duration, Instant};

use solana_sdk::pubkey::Pubkey;

use super::{VaultAccount, VaultPhase, VaultSafety};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapitalStep {
    CloseDeposits,
    OpenDeposits,
    ComputeEquity,
    ProcessWithdrawals,
    ProcessDeposits,
    FinalizeCycle,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WindowClock {
    pub open_for: Option<Duration>,
    pub closed_for: Option<Duration>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WindowTracker {
    state: Option<(bool, Instant)>,
}

impl WindowTracker {
    pub fn observe(&mut self, open: bool, now: Instant) -> WindowClock {
        let since = match self.state {
            Some((was_open, since)) if was_open == open => since,
            _ => {
                self.state = Some((open, now));
                now
            }
        };
        let held = now.saturating_duration_since(since);
        if open {
            WindowClock {
                open_for: Some(held),
                closed_for: None,
            }
        } else {
            WindowClock {
                open_for: None,
                closed_for: Some(held),
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPolicy {
    pub reopen: bool,
    pub deposit_window: Duration,
    pub trading_window: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultView {
    pub phase: VaultPhase,
    pub safety: VaultSafety,
    pub equity_computed: bool,
    pub accepting_deposits: bool,
    pub pending_deposits: u32,
    pub pending_withdrawals: u32,
}

impl From<&VaultAccount> for VaultView {
    fn from(vault: &VaultAccount) -> Self {
        Self {
            phase: vault.phase,
            safety: vault.safety,
            equity_computed: vault.equity_computed,
            accepting_deposits: vault.accepting_deposits,
            pending_deposits: vault.pending_deposit_count,
            pending_withdrawals: vault.pending_withdraw_count,
        }
    }
}

#[must_use]
pub fn next_step(
    view: VaultView,
    window: WindowClock,
    policy: WindowPolicy,
) -> Option<CapitalStep> {
    use VaultPhase::{Idle, ProcessingCapital};
    use VaultSafety::{Closing, Frozen, Normal, WithdrawOnly};

    let pending = view.pending_deposits > 0 || view.pending_withdrawals > 0;
    match (view.safety, view.phase) {
        // Terminal payouts must not wait behind an unpayable deposit refund.
        (WithdrawOnly, Idle) if view.pending_withdrawals > 0 => {
            Some(CapitalStep::ProcessWithdrawals)
        }
        (Frozen | Closing | WithdrawOnly, _) if view.pending_deposits > 0 => {
            Some(CapitalStep::ProcessDeposits)
        }
        (Normal | Frozen, ProcessingCapital) => Some(if !view.equity_computed {
            CapitalStep::ComputeEquity
        } else if view.safety == Normal && view.pending_deposits > 0 {
            CapitalStep::ProcessDeposits
        } else if view.pending_withdrawals > 0 {
            CapitalStep::ProcessWithdrawals
        } else {
            CapitalStep::FinalizeCycle
        }),
        (Frozen, Idle) if view.pending_withdrawals > 0 => Some(if view.equity_computed {
            CapitalStep::ProcessWithdrawals
        } else {
            CapitalStep::ComputeEquity
        }),
        (Normal, Idle) if !pending && view.accepting_deposits => window
            .open_for
            .is_some_and(|open| open >= policy.deposit_window)
            .then_some(CapitalStep::CloseDeposits),
        (Normal, Idle) if !pending => (policy.reopen
            && !view.equity_computed
            && window
                .closed_for
                .is_some_and(|closed| closed >= policy.trading_window))
        .then_some(CapitalStep::OpenDeposits),
        (Normal, Idle) if view.accepting_deposits => Some(CapitalStep::CloseDeposits),
        (Normal, Idle) if !view.equity_computed => Some(CapitalStep::ComputeEquity),
        (Normal, Idle) if view.pending_deposits > 0 => Some(CapitalStep::ProcessDeposits),
        (Normal, Idle) => Some(CapitalStep::ProcessWithdrawals),
        _ => None,
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WithdrawalPlan {
    pub partial: Vec<Pubkey>,
    /// Sole remaining holder, redeemed through `ProcessFinalWithdrawal`.
    pub final_user: Option<Pubkey>,
}

/// A queue that would drain the share supply holds its largest request back for the final redemption.
#[must_use]
pub fn plan_withdrawals(requests: &[(Pubkey, u64)], total_shares: u64) -> WithdrawalPlan {
    let mut plan = WithdrawalPlan::default();
    if let [(user, shares)] = requests
        && *shares == total_shares
    {
        plan.final_user = Some(*user);
        return plan;
    }
    let queued = requests
        .iter()
        .fold(0u64, |sum, (_, shares)| sum.saturating_add(*shares));
    let mut ordered = requests.to_vec();
    if !ordered.is_empty() && queued >= total_shares {
        ordered.sort_by_key(|(_, shares)| std::cmp::Reverse(*shares));
        ordered.remove(0);
    }
    plan.partial = ordered.into_iter().map(|(user, _)| user).collect();
    plan
}
