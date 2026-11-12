use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};
use std::time::SystemTime;

use crate::Address32;
use crate::types::unix_time::UnixSecondsString;

pub use crate::types::VaultLeg;
use crate::ws::types::{MarketInfo, TokenInfo};

/// Public `/api/v1/vaults` row. Amounts and times arrive as decimal strings and decode to numbers.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Vault {
    pub vault_pda: Address32,
    pub vault_id: String,
    pub governance: Address32,
    pub fee_recipient: Address32,
    pub primary_delegate: Address32,
    pub main_mint: Address32,
    pub second_mint: Address32,
    pub share_mint: Address32,
    pub main_token_program: TokenProgramKind,
    pub second_token_program: TokenProgramKind,
    pub main_decimals: u8,
    pub second_decimals: u8,
    /// `idle`, `active`, `settling` or `processing_capital`.
    pub phase: String,
    /// `normal`, `frozen`, `emergency_unwind`, `withdraw_only` or `closing`.
    pub safety: String,
    pub capabilities: u32,
    #[serde_as(as = "UnixSecondsString")]
    pub active_expiry_ts: SystemTime,
    #[serde_as(as = "DisplayFromStr")]
    pub open_positions: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub total_shares: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub share_price: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub equity: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub capital_generation: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub second_leg_generation: u64,
    pub accepting_deposits: bool,
    pub equity_computed: bool,
    pub pending_deposit_count: u32,
    pub pending_withdraw_count: u32,
    #[serde_as(as = "DisplayFromStr")]
    pub pending_deposit_amount: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub pending_withdraw_shares: u64,
    pub performance_fee_bps: u16,
    pub management_fee_bps: u16,
    pub manager_stake_min_bps: u16,
    #[serde_as(as = "DisplayFromStr")]
    pub share_price_hwm: u64,
    #[serde_as(as = "UnixSecondsString")]
    pub last_hwm_update_at: SystemTime,
    #[serde_as(as = "UnixSecondsString")]
    pub last_hwm_reset_at: SystemTime,
    #[serde_as(as = "UnixSecondsString")]
    pub last_settlement_at: SystemTime,
    pub capacity: VaultCapacity,
    #[serde_as(as = "DisplayFromStr")]
    pub capital_fallback_delay_seconds: u64,
    pub deposit_window: DepositWindow,
    pub balances: VaultBalances,

    pub staging: VaultStaging,
    /// The next stage carries `staged_generation + 1`.
    #[serde_as(as = "DisplayFromStr")]
    pub staged_generation: u64,
    pub policy: VaultPolicy,
    pub projection: ProjectionCursor,
    pub tvl_usd: VaultTvl,
    pub stats: VaultStatsState,
    pub metadata: VaultMetadataState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultMetadata {
    pub name: String,

    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub curator: Option<String>,

    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub strategy: Option<String>,

    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub description: Option<String>,

    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub website: Option<String>,
    /// X handle without `@`.

    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub twitter: Option<String>,

    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub logo_url: Option<String>,
}

/// Leg balance moved into a vault ATA of another mint, pending return to a leg.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VaultStaging {
    None {},
    Staged {
        mint: Address32,
        from_leg: VaultLeg,
        #[serde_as(as = "DisplayFromStr")]
        amount: u64,
        #[serde_as(as = "UnixSecondsString")]
        staged_at: SystemTime,
        /// The open slot's generation; an unstage names it.
        #[serde_as(as = "DisplayFromStr")]
        generation: u64,
    },
}

#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DepositWindow {
    Unset {},
    Set {
        #[serde_as(as = "UnixSecondsString")]
        set_at: SystemTime,
    },
}

/// Live SPL balances of the vault accounts, from the backend's balance watcher.
#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VaultBalances {
    Unobserved {},
    Observed {
        #[serde_as(as = "DisplayFromStr")]
        main: u64,
        #[serde_as(as = "DisplayFromStr")]
        second: u64,
        #[serde_as(as = "DisplayFromStr")]
        live_share_supply: u64,
        #[serde_as(as = "DisplayFromStr")]
        manager_stake_shares: u64,
        #[serde_as(as = "UnixSecondsString")]
        observed_at: SystemTime,
        #[serde_as(as = "DisplayFromStr")]
        slot: u64,
    },
}

/// `equity` at the main mint's USD oracle price, 1e9 scale.
#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VaultTvl {
    Unpriced {},
    Priced {
        #[serde_as(as = "DisplayFromStr")]
        value_1e9: u128,
        #[serde_as(as = "UnixSecondsString")]
        price_published_at: SystemTime,
    },
}

/// A derived amount the server could not compute (missing decimals, price or overflow).
#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Atoms<T>
where
    T: std::fmt::Display + std::str::FromStr,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    Unavailable {},
    Available {
        #[serde_as(as = "DisplayFromStr")]
        atoms: T,
    },
}

impl<T> Atoms<T>
where
    T: std::fmt::Display + std::str::FromStr,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    pub fn available(self) -> Option<T> {
        match self {
            Self::Unavailable {} => None,
            Self::Available { atoms } => Some(atoms),
        }
    }
}

/// A metric over a window the vault's history does not cover yet.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Metric<T> {
    InsufficientHistory {},
    Available { value: T },
}

/// A historical fact the projection did not record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transaction {
    Unknown {},
    Known { signature: String },
}

/// Indexed statistics; `behind` while they are for an older capital generation.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VaultStatsState {
    NotIndexed {},
    Behind {
        #[serde_as(as = "UnixSecondsString")]
        created_at: SystemTime,
    },
    Current {
        #[serde_as(as = "UnixSecondsString")]
        created_at: SystemTime,
        #[serde_as(as = "UnixSecondsString")]
        computed_at: SystemTime,
        generation: VaultGeneration,
    },
}

/// The current capital generation; `none` until its first checkpoint is indexed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VaultGeneration {
    None {},
    Covered {
        started: GenerationStart,
        premium: Vec<VaultPremium>,
        fees: VaultFees,
        depositors: u64,
        performance: Box<VaultPerformance>,
    },
}

#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum GenerationStart {
    NoCheckpoint {},
    At {
        #[serde_as(as = "UnixSecondsString")]
        at: SystemTime,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VaultMetadataState {
    None {},
    Set(VaultMetadata),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenProgramKind {
    #[serde(rename = "legacy_spl")]
    LegacySpl,
    #[serde(rename = "token_2022")]
    Token2022,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultPolicy {
    pub max_open_positions: u16,
    pub max_rebalance_drift_bps: u16,
    pub min_main_liquidity_bps: u16,
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionCursor {
    #[serde_as(as = "DisplayFromStr")]
    pub version: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub slot: u64,
    pub transaction_index: u32,
    pub signature: String,
    pub event_index: u32,
}

/// A zero on-chain cap means the vault accepts deposits without a limit.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VaultCapacity {
    Unlimited {},
    Limited {
        #[serde_as(as = "DisplayFromStr")]
        cap_limit: u64,
        #[serde_as(as = "DisplayFromStr")]
        remaining: u64,
    },
}

/// Net premium per quote mint, in quote atoms, for the current cycle and generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultPremium {
    pub quote_mint: Address32,
    pub cycle: PremiumFlow,
    pub generation: PremiumFlow,
}

#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PremiumFlow {
    #[serde_as(as = "DisplayFromStr")]
    pub received: u128,
    /// `unavailable` when a holder leg's market lacks decimals or its open price.
    pub paid: Atoms<u128>,
}

/// Generation totals in main atoms.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultFees {
    #[serde_as(as = "DisplayFromStr")]
    pub management_fee_assessed: u128,
    #[serde_as(as = "DisplayFromStr")]
    pub performance_fee_assessed: u128,
    #[serde_as(as = "DisplayFromStr")]
    pub realized_fee_value: u128,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultPerformance {
    pub returns: VaultReturns,
    pub apy_30d: Metric<f64>,
    pub apy_90d: Metric<f64>,
    pub max_drawdown_pct: Metric<f64>,
    pub worst_cycle_return_pct: Metric<f64>,
    pub losing_cycles: Metric<u64>,
    pub cycles_completed: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultReturns {
    #[serde(rename = "7d")]
    pub d7: WindowReturn,
    #[serde(rename = "30d")]
    pub d30: WindowReturn,
    #[serde(rename = "90d")]
    pub d90: WindowReturn,
    pub all: WindowReturn,
}

/// Net-of-fee booked share price return between two checkpoint times.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum WindowReturn {
    InsufficientHistory {},
    Available {
        return_pct: f64,
        #[serde_as(as = "UnixSecondsString")]
        window_start_at: SystemTime,
        #[serde_as(as = "UnixSecondsString")]
        window_end_at: SystemTime,
    },
}

/// Public `/api/v1/vaults/{pda}/positions` row.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultPosition {
    pub vault_pda: Address32,
    pub position_pda: Address32,
    pub order_id: String,
    pub role: VaultPositionRole,
    pub market_pda: Address32,
    pub position_type: VaultPositionType,
    /// The lower strike for spreads.
    #[serde_as(as = "DisplayFromStr")]
    pub strike: u64,
    pub product: VaultPositionProduct,
    #[serde_as(as = "DisplayFromStr")]
    pub quantity: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub net_premium: u64,
    pub counterparty_owner: Address32,
    /// Base units of the funding mint; `unavailable` when the funding was not recorded.
    pub funded_amount: Atoms<u64>,
    pub status: VaultPositionStatus,
    pub close: PositionClose,
    #[serde_as(as = "UnixSecondsString")]
    pub created_at: SystemTime,
    #[serde_as(as = "UnixSecondsString")]
    pub acquired_at: SystemTime,
    pub transaction: Transaction,
}

#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PositionClose {
    Open {},
    Closed {
        #[serde_as(as = "UnixSecondsString")]
        closed_at: SystemTime,
        is_otm: bool,
    },
}

/// Physical CC/CSP, or a cash spread with its upper strike and the quote atoms it reserves.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum VaultPositionProduct {
    Physical {},
    Spread {
        #[serde_as(as = "DisplayFromStr")]
        upper_strike: u64,
        #[serde_as(as = "DisplayFromStr")]
        collateral: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultPositionRole {
    Writer,
    Holder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultPositionType {
    CoveredCall,
    CashSecuredPut,
    CallSpread,
    PutSpread,
    CashPut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultPositionStatus {
    Open,
    Funded,
    Liquidated,
    Settled,
}

/// Public `/api/v1/vaults/{pda}/exposure`: outstanding positions at the indicative spot.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultExposure {
    #[serde(deserialize_with = "crate::ws::types::deserialize_required_nullable")]
    pub next_cursor: Option<String>,
    pub vault_pda: Address32,
    /// Unix seconds the positions were read; never older than 15 s.
    #[serde_as(as = "UnixSecondsString")]
    pub as_of: SystemTime,
    pub positions: Vec<VaultExposurePosition>,
    pub totals: Vec<VaultExposureTotal>,
}

/// `intrinsic` is in quote atoms: positive for holder legs, negative for writer legs.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultExposurePosition {
    pub position_pda: Address32,
    pub market_pda: Address32,
    pub position_type: VaultPositionType,
    pub role: VaultPositionRole,
    pub status: VaultPositionStatus,
    #[serde_as(as = "DisplayFromStr")]
    pub strike: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub quantity: u64,
    #[serde_as(as = "UnixSecondsString")]
    pub expiry_ts: SystemTime,
    pub underlying_mint: Address32,
    pub quote_mint: Address32,
    pub decimals: ExposureDecimals,
    pub spot: ExposureSpot,
    pub moneyness: Moneyness,
    pub intrinsic: Atoms<i128>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExposureDecimals {
    Unknown {},
    Known { underlying: u8, quote: u8 },
}

/// Underlying price in quote, 1e9 scale, from the oracle cache.
#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExposureSpot {
    Unpriced {},
    Priced {
        #[serde_as(as = "DisplayFromStr")]
        price: u64,
        #[serde_as(as = "UnixSecondsString")]
        published_at: SystemTime,
    },
}

/// Physical positions against a priced spot; spreads have none.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Moneyness {
    Unavailable {},
    Available { distance_pct: f64, itm: bool },
}

/// `unavailable` when any position of the quote mint lacks the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultExposureTotal {
    pub quote_mint: Address32,
    pub notional_at_strike: Atoms<u128>,
    pub outstanding_intrinsic: Atoms<i128>,
}

/// Public `/api/v1/vaults/{pda}/cycles` row.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VaultCycle {
    #[serde_as(as = "DisplayFromStr")]
    pub cycle_number: u64,
    pub capital_generation: CycleGeneration,
    #[serde_as(as = "UnixSecondsString")]
    pub expiry_ts: SystemTime,
    #[serde_as(as = "UnixSecondsString")]
    pub started_at: SystemTime,
    #[serde_as(as = "DisplayFromStr")]
    pub share_price_start: u64,
    #[serde_as(as = "DisplayFromStr")]
    pub equity_start: u64,
    pub state: CycleState,
    pub positions_opened: u32,
    pub positions_settled: u32,
    #[serde_as(as = "DisplayFromStr")]
    pub performance_fee_assessed: u128,
    #[serde_as(as = "DisplayFromStr")]
    pub management_fee_assessed: u128,
}

/// `unknown` for cycles recorded before generations were tracked.
#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CycleGeneration {
    Unknown {},
    Known {
        #[serde_as(as = "DisplayFromStr")]
        generation: u64,
    },
}

#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CycleState {
    Running {},
    /// Equity is computed at the boundary; the cycle is not finalized yet.
    Priced {
        #[serde_as(as = "DisplayFromStr")]
        share_price_end: u64,
        #[serde_as(as = "DisplayFromStr")]
        equity_end: u64,
        return_pct: CycleReturn,
    },
    Finished {
        #[serde_as(as = "UnixSecondsString")]
        finished_at: SystemTime,
        #[serde_as(as = "DisplayFromStr")]
        share_price_end: u64,
        #[serde_as(as = "DisplayFromStr")]
        equity_end: u64,
        /// `unavailable` when the start share price is zero.
        return_pct: CycleReturn,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CycleReturn {
    Unavailable {},
    Available { value: f64 },
}

/// Public `/api/v1/vaults/{pda}/deposit-requests` row.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultDepositRequest {
    pub wallet: Address32,
    #[serde_as(as = "DisplayFromStr")]
    pub amount: u64,
    #[serde_as(as = "UnixSecondsString")]
    pub created_at: SystemTime,
    pub transaction: Transaction,
}

/// Public `/api/v1/vaults/{pda}/withdraw-requests` row.
#[serde_as]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultWithdrawRequest {
    pub wallet: Address32,
    #[serde_as(as = "DisplayFromStr")]
    pub shares: u64,
    #[serde_as(as = "UnixSecondsString")]
    pub created_at: SystemTime,
    pub transaction: Transaction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct VaultCyclesResponse {
    pub(super) cycles: Vec<VaultCycle>,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub(super) next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct VaultRequestsResponse<T> {
    pub(super) requests: Vec<T>,
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub(super) next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct VaultPositionsResponse {
    pub(super) positions: Vec<VaultPosition>,
    pub(super) next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct VaultsResponse {
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub next_cursor: Option<String>,
    pub(super) vaults: Vec<Vault>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MarketsResponse {
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub next_cursor: Option<String>,
    pub(super) markets: Vec<MarketInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TokensResponse {
    #[serde(deserialize_with = "crate::wire::deserialize_required_option")]
    pub next_cursor: Option<String>,
    pub(super) tokens: Vec<TokenInfo>,
}

/// A configured count bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CountLimit {
    Unlimited {},
    Limited { value: u64 },
}

/// Public `/api/v1/venue`: limits the server validates against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Venue {
    pub program_id: crate::Address32,
    pub min_quote_validity_seconds: u32,
    pub settlement_buffer_seconds: u32,
    pub quote_refresh_lead_seconds: u32,
    pub min_rfq_timeout_seconds: u32,
    pub max_rfq_timeout_seconds: u32,
    pub signature_timeout_seconds: u64,
    pub max_quotes_per_rfq: CountLimit,
    pub max_active_rfqs_per_taker: CountLimit,
    pub pre_expiry_cutoff_seconds: u64,
    /// Largest `limit` any paged request accepts.
    pub max_page_size: u32,
    /// Most quotes one `BatchQuotes` message may carry.
    pub max_batch_quotes: u32,
}
