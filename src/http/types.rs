use serde::{Deserialize, Serialize};

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
    pub capital_fallback_delay_secs: String,
    pub deposit_window_set_ts: Option<String>,
    pub main_balance: Option<String>,
    pub second_balance: Option<String>,
    pub balance_updated_at: Option<String>,
    pub live_share_supply: Option<String>,
    pub observed_manager_stake_shares: Option<String>,
    pub balance_chain_slot: Option<String>,
    pub policy: VaultPolicy,
    pub projection: ProjectionCursor,
    pub performance: VaultPerformance,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
#[non_exhaustive]
pub enum VaultPerformance {
    Unavailable,
    NoTrackRecord,
    ShortTrackRecord(TrackRecord),
    Annualized {
        #[serde(flatten)]
        track_record: TrackRecord,
        annualized_return_pct: f64,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TrackRecord {
    pub cycles_completed: u32,
    pub last_cycle_return_pct: f64,
    pub cumulative_return_pct: f64,
    pub track_record_secs: String,
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

/// Public `/api/v1/venue`: limits rfq-server validates against. `None` limits are unbounded.
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
