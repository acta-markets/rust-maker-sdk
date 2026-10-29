use sha2::{Digest, Sha256};

use super::{ORDER_DOMAIN_TAG, ORDER_ID_LEN, SignerLike, sign_order_id_with_signer};
use crate::spreads::{SpreadError, SpreadType, validate_premium};
use crate::types::TakerSide;

/// Spread layout: the physical layout under the shared `ACTA` domain, then upper strike.
pub const SPREAD_ORDER_PREIMAGE_LEN: usize = 190;

#[derive(Debug, Clone)]
pub struct SpreadOrderPreimageArgs {
    pub chain_id: u64,
    pub program_id: [u8; 32],
    pub side: TakerSide,
    pub position_type: SpreadType,
    pub market: [u8; 32],
    pub lower_strike: u64,
    pub upper_strike: u64,
    pub quantity: u64,
    pub gross_price: u64,
    pub valid_until: u64,
    pub maker: [u8; 32],
    pub taker: [u8; 32],
    pub nonce: u64,
}

pub fn build_spread_order_preimage(
    args: &SpreadOrderPreimageArgs,
) -> Result<[u8; SPREAD_ORDER_PREIMAGE_LEN], SpreadError> {
    validate_premium(
        args.position_type,
        args.lower_strike,
        args.upper_strike,
        args.quantity,
        args.gross_price,
    )?;
    let mut bytes = [0; SPREAD_ORDER_PREIMAGE_LEN];
    bytes[..4].copy_from_slice(&ORDER_DOMAIN_TAG);
    bytes[4..12].copy_from_slice(&args.chain_id.to_le_bytes());
    bytes[12..44].copy_from_slice(&args.program_id);
    bytes[44] = u8::from(args.side.is_taker_buy());
    bytes[45] = args.position_type as u8;
    bytes[46..78].copy_from_slice(&args.market);
    for (offset, value) in [
        (78, args.lower_strike),
        (86, args.quantity),
        (94, args.gross_price),
        (102, args.valid_until),
        (174, args.nonce),
        (182, args.upper_strike),
    ] {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    bytes[110..142].copy_from_slice(&args.maker);
    bytes[142..174].copy_from_slice(&args.taker);
    Ok(bytes)
}

pub fn compute_spread_order_id(
    args: &SpreadOrderPreimageArgs,
) -> Result<[u8; ORDER_ID_LEN], SpreadError> {
    Ok(Sha256::digest(build_spread_order_preimage(args)?).into())
}

pub fn sign_spread_order<S: SignerLike + ?Sized>(
    args: &SpreadOrderPreimageArgs,
    signer: &S,
) -> Result<([u8; ORDER_ID_LEN], [u8; 64]), SpreadError> {
    if args.side.is_taker_buy() {
        return Err(SpreadError::TakerBuyUnsupported);
    }
    let order_id = compute_spread_order_id(args)?;
    Ok((order_id, sign_order_id_with_signer(&order_id, signer)))
}
