use super::ChainError;
use crate::spreads::SpreadType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSpreadEvent {
    pub user: [u8; 32],
    pub maker_pda: [u8; 32],
    pub market_pda: [u8; 32],
    pub position_pda: [u8; 32],
    pub position_type: SpreadType,
    pub strike: u64,
    pub upper_strike: u64,
    pub quantity: u64,
    pub price: u64,
    pub valid_until: u64,
    pub order_id: [u8; 32],
    pub total_premium: u64,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettleSpreadEvent {
    pub settler: [u8; 32],
    pub taker: [u8; 32],
    pub maker_owner: [u8; 32],
    pub position_pda: [u8; 32],
    pub market_pda: [u8; 32],
    pub final_status: u8,
    pub is_otm: bool,
    pub maker_payoff: u64,
    pub taker_return: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpreadEvent {
    Open(OpenSpreadEvent),
    Settle(SettleSpreadEvent),
}

/// Decode an ACTA binary envelope. Other event kinds return None.
pub fn decode_spread_event(data: &[u8]) -> Result<Option<SpreadEvent>, ChainError> {
    if data.len() < 6 || &data[..5] != b"ACTA\x01" {
        return Err(ChainError::InvalidAccountData);
    }
    let bytes = &data[6..];
    let array = |offset: usize| -> Result<[u8; 32], ChainError> {
        bytes
            .get(offset..offset + 32)
            .and_then(|s| s.try_into().ok())
            .ok_or(ChainError::InvalidAccountData)
    };
    let word = |offset: usize| -> Result<u64, ChainError> {
        let value = bytes
            .get(offset..offset + 8)
            .and_then(|s| s.try_into().ok())
            .ok_or(ChainError::InvalidAccountData)?;
        Ok(u64::from_le_bytes(value))
    };
    Ok(Some(match data[5] {
        79 => {
            if bytes.len() != 217 {
                return Err(ChainError::InvalidAccountData);
            }
            SpreadEvent::Open(OpenSpreadEvent {
                user: array(0)?,
                maker_pda: array(32)?,
                market_pda: array(64)?,
                position_pda: array(96)?,
                position_type: SpreadType::try_from(bytes[128])?,
                strike: word(129)?,
                quantity: word(137)?,
                price: word(145)?,
                valid_until: word(153)?,
                order_id: array(161)?,
                total_premium: word(193)?,
                created_at: word(201)?,
                upper_strike: word(209)?,
            })
        }
        80 => {
            if bytes.len() != 178 || bytes[160] != 4 || bytes[161] > 1 {
                return Err(ChainError::InvalidAccountData);
            }
            SpreadEvent::Settle(SettleSpreadEvent {
                settler: array(0)?,
                taker: array(32)?,
                maker_owner: array(64)?,
                position_pda: array(96)?,
                market_pda: array(128)?,
                final_status: bytes[160],
                is_otm: bytes[161] == 1,
                maker_payoff: word(162)?,
                taker_return: word(170)?,
            })
        }
        _ => return Ok(None),
    }))
}
