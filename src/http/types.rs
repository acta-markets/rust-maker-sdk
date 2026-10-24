use serde::{Deserialize, Serialize};

pub use crate::types::VaultLeg;
use crate::ws::types::{MarketInfo, TokenInfo};

/// Public `/api/v1/vaults` row. Integers wider than JSON-safe stay decimal strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Vault {
    pub vault_pda: String,
    pub vault_id: String,
    pub governance: String,
    pub fee_recipient: String,
    pub primary_delegate: String,
    pub main_mint: String,
    pub second_mint: String,
    pub share_mint: String,
    pub main_token_program: TokenProgramKind,
    pub second_token_program: TokenProgramKind,
    pub main_decimals: u8,
    pub second_decimals: u8,
    /// `idle`, `active`, `settling` or `processing_capital`.
    pub phase: String,
    /// `normal`, `frozen`, `emergency_unwind`, `withdraw_only` or `closing`.
    pub safety: String,
    pub capabilities: u32,
    pub active_expiry: String,
    pub open_positions: String,
    pub total_shares: String,
    pub share_price: String,
    pub equity: String,
    pub capital_generation: String,
    pub second_leg_generation: String,
    pub accepting_deposits: bool,
    pub equity_computed: bool,
    pub pending_deposit_count: u32,
    pub pending_withdraw_count: u32,
    pub pending_deposit_amount: String,
    pub pending_withdraw_shares: String,
    pub performance_fee_bps: u16,
    pub management_fee_bps: u16,
    pub manager_stake_min_bps: u16,
    pub share_price_hwm: String,
    pub last_hwm_update_ts: String,
    pub last_hwm_reset_ts: String,
    pub last_settlement_ts: String,
    pub cap_limit: String,
    pub capacity: VaultCapacity,
    pub capital_fallback_delay_secs: String,
    pub deposit_window_set_ts: Option<String>,
    pub main_balance: Option<String>,
    pub second_balance: Option<String>,
    pub balance_updated_at: Option<String>,
    pub live_share_supply: Option<String>,
    pub observed_manager_stake_shares: Option<String>,
    pub balance_chain_slot: Option<String>,
    #[serde(default)]
    pub staging: Option<VaultStaging>,
    /// The next stage carries `staged_generation + 1`.
    pub staged_generation: String,
    pub policy: VaultPolicy,
    pub projection: ProjectionCursor,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub generation_started_at: Option<String>,
    /// Indicative: booked equity at the main mint's USD price, 1e9 scale.
    #[serde(default)]
    pub tvl_usd_1e9: Option<String>,
    #[serde(default)]
    pub price_time: Option<String>,
    #[serde(default)]
    pub premium: Option<Vec<VaultPremium>>,
    #[serde(default)]
    pub fees: Option<VaultFees>,
    #[serde(default)]
    pub depositors_in_generation: Option<u64>,
    #[serde(default)]
    pub performance: Option<VaultPerformance>,
    /// Display only; `governance` is the authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<VaultMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VaultMetadata {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    /// X handle without `@`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twitter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo_url: Option<String>,
}

/// Leg balance moved into a vault ATA of another mint, pending return to a leg.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VaultStaging {
    pub mint: String,
    pub from_leg: VaultLeg,
    pub amount: String,
    pub staged_at: String,
    /// The open slot's generation; an unstage names it.
    pub generation: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TokenProgramKind {
    #[serde(rename = "legacy_spl")]
    LegacySpl,
    #[serde(rename = "token_2022")]
    Token2022,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VaultPolicy {
    pub max_open_positions: u16,
    pub max_rebalance_drift_bps: u16,
    pub min_main_liquidity_bps: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProjectionCursor {
    pub version: String,
    pub slot: String,
    pub transaction_index: u32,
    pub signature: String,
    pub event_index: u32,
}

/// `remaining` is `None` when uncapped (`cap_limit` is 0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VaultCapacity {
    pub cap_limit: String,
    #[serde(default)]
    pub remaining: Option<String>,
}

/// Net premium per quote mint, in quote atoms, for the current cycle and generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VaultPremium {
    pub quote_mint: String,
    pub cycle: PremiumFlow,
    pub generation: PremiumFlow,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PremiumFlow {
    pub received: String,
    /// `None` when a holder leg's market lacks decimals or its open price.
    pub paid: Option<String>,
}

/// Generation totals in main atoms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VaultFees {
    pub management_fee_assessed: String,
    pub performance_fee_assessed: String,
    pub realized_fee_value: String,
}

/// Metrics of `capital_generation` as of `computed_at`; all `None` unless `covered`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VaultPerformance {
    pub capital_generation: String,
    pub computed_at: String,
    pub covered: bool,
    #[serde(default)]
    pub returns: VaultReturns,
    #[serde(default)]
    pub apy_30d: Option<f64>,
    #[serde(default)]
    pub apy_90d: Option<f64>,
    #[serde(default)]
    pub max_drawdown_pct: Option<f64>,
    #[serde(default)]
    pub worst_cycle_return_pct: Option<f64>,
    #[serde(default)]
    pub losing_cycles: Option<u64>,
    #[serde(default)]
    pub cycles_completed: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VaultReturns {
    #[serde(rename = "7d", default)]
    pub d7: Option<WindowReturn>,
    #[serde(rename = "30d", default)]
    pub d30: Option<WindowReturn>,
    #[serde(rename = "90d", default)]
    pub d90: Option<WindowReturn>,
    #[serde(default)]
    pub all: Option<WindowReturn>,
}

/// Net-of-fee booked share price return between two checkpoint times.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WindowReturn {
    pub return_pct: f64,
    pub from_time: String,
    pub to_time: String,
}

/// Public `/api/v1/vaults/{pda}/positions` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VaultPosition {
    pub vault: String,
    pub position: String,
    pub order_id: String,
    pub role: VaultPositionRole,
    pub market: String,
    pub position_type: VaultPositionType,
    pub strike: String,
    pub quantity: String,
    pub net_premium: String,
    pub counterparty: String,
    pub funded_amount: Option<String>,
    pub status: VaultPositionStatus,
    pub is_otm: Option<bool>,
    pub created_at: String,
    pub acquired_at: String,
    pub settled_at: Option<String>,
    pub transaction_signature: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum VaultPositionRole {
    Writer,
    Holder,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum VaultPositionType {
    CoveredCall,
    CashSecuredPut,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum VaultPositionStatus {
    Open,
    Funded,
    Liquidated,
    Settled,
    #[serde(other)]
    Unknown,
}

/// Public `/api/v1/vaults/{pda}/exposure`: outstanding positions at the indicative spot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VaultExposure {
    pub vault_pda: String,
    /// Unix seconds the positions were read; never older than 15 s.
    pub as_of: String,
    pub positions: Vec<VaultExposurePosition>,
    pub totals: Vec<VaultExposureTotal>,
}

/// `intrinsic` is in quote atoms: positive for holder legs, negative for writer legs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VaultExposurePosition {
    pub position: String,
    pub market: String,
    pub position_type: VaultPositionType,
    pub role: VaultPositionRole,
    pub status: VaultPositionStatus,
    pub strike: String,
    pub quantity: String,
    pub expiry: String,
    pub underlying_mint: String,
    pub quote_mint: String,
    #[serde(default)]
    pub underlying_decimals: Option<u8>,
    #[serde(default)]
    pub quote_decimals: Option<u8>,
    #[serde(default)]
    pub spot: Option<String>,
    #[serde(default)]
    pub spot_time: Option<String>,
    #[serde(default)]
    pub distance_pct: Option<f64>,
    #[serde(default)]
    pub itm: Option<bool>,
    #[serde(default)]
    pub intrinsic: Option<String>,
}

/// `None` when any position of the quote mint lacks the value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VaultExposureTotal {
    pub quote_mint: String,
    #[serde(default)]
    pub notional_at_strike: Option<String>,
    #[serde(default)]
    pub outstanding_intrinsic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct VaultPositionsResponse {
    pub(super) positions: Vec<VaultPosition>,
    pub(super) next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct VaultsResponse {
    pub(super) vaults: Vec<Vault>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MarketsResponse {
    pub(super) markets: Vec<MarketInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TokensResponse {
    pub(super) tokens: Vec<TokenInfo>,
}

/// Public `/api/v1/venue`: limits the server validates against. `None` limits are unbounded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Venue {
    pub program_id: String,
    pub min_quote_validity_seconds: u32,
    pub settlement_buffer_seconds: u32,
    pub quote_refresh_lead_seconds: u32,
    pub min_rfq_timeout_seconds: u32,
    pub max_rfq_timeout_seconds: u32,
    pub signature_timeout_seconds: u64,
    pub max_quotes_per_rfq: Option<u64>,
    pub max_active_rfqs_per_taker: Option<u64>,
    pub pre_expiry_cutoff_seconds: u64,
}
