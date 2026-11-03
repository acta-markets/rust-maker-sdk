use super::ChainError;
use crate::types::{OptionTerms, Strike, TakerSide};

/// The one event every open emits, whoever writes and holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionOpenedEvent {
    pub position_pda: [u8; 32],
    pub market_pda: [u8; 32],
    pub order_id: [u8; 32],
    pub writer: [u8; 32],
    pub holder: [u8; 32],
    /// `Position.flags`: bit 0 a vault writes, bit 1 a vault holds, bit 2 the taker bought.
    pub flags: u8,
    pub side: TakerSide,
    pub terms: OptionTerms,
    pub quantity: u64,
    pub price: u64,
    pub gross_premium: u64,
    pub net_premium: u64,
    pub collateral: u64,
    pub valid_until: u64,
    pub created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettleSpreadEvent {
    pub settler: [u8; 32],
    pub writer: [u8; 32],
    pub holder: [u8; 32],
    pub position_pda: [u8; 32],
    pub market_pda: [u8; 32],
    pub final_status: u8,
    pub is_otm: bool,
    pub holder_payoff: u64,
    pub writer_return: u64,
}

/// The position events this module decodes; other kinds are skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PositionEvent {
    Opened(PositionOpenedEvent),
    SpreadSettled(SettleSpreadEvent),
}

const HEADER: &[u8; 5] = b"ACTA\x02";
const POSITION_OPENED: u8 = 11;
const SETTLE_SPREAD: u8 = 79;
const FLAG_TAKER_BUY: u8 = 1 << 2;

/// Decode an ACTA version-2 envelope. Other event kinds return None.
pub fn decode_position_event(data: &[u8]) -> Result<Option<PositionEvent>, ChainError> {
    if data.len() < 6 || &data[..5] != HEADER {
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
        POSITION_OPENED => {
            if bytes.len() != 234 {
                return Err(ChainError::InvalidAccountData);
            }
            let flags = bytes[160];
            let product = crate::ChainPositionType::try_from(bytes[161])
                .map_err(|error| ChainError::InvalidPositionType(error.0))?;
            let strike = Strike::new(word(162)?);
            let upper = product
                .is_spread()
                .then(|| word(170))
                .transpose()?
                .map(Strike::new);
            PositionEvent::Opened(PositionOpenedEvent {
                position_pda: array(0)?,
                market_pda: array(32)?,
                order_id: array(64)?,
                writer: array(96)?,
                holder: array(128)?,
                flags,
                side: if flags & FLAG_TAKER_BUY != 0 {
                    TakerSide::Buy
                } else {
                    TakerSide::Sell
                },
                terms: OptionTerms::from_chain(crate::ChainTerms::from_parts(
                    product, strike, upper,
                )?),
                quantity: word(178)?,
                price: word(186)?,
                gross_premium: word(194)?,
                net_premium: word(202)?,
                collateral: word(210)?,
                valid_until: word(218)?,
                created_at: word(226)?,
            })
        }
        SETTLE_SPREAD => {
            if bytes.len() != 178 || bytes[160] != 4 || bytes[161] > 1 {
                return Err(ChainError::InvalidAccountData);
            }
            PositionEvent::SpreadSettled(SettleSpreadEvent {
                settler: array(0)?,
                writer: array(32)?,
                holder: array(64)?,
                position_pda: array(96)?,
                market_pda: array(128)?,
                final_status: bytes[160],
                is_otm: bytes[161] == 1,
                holder_payoff: word(162)?,
                writer_return: word(170)?,
            })
        }
        _ => return Ok(None),
    }))
}
