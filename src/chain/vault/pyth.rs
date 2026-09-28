use solana_sdk::pubkey::Pubkey;

/// Pyth push-oracle program that owns sponsored price-update accounts.
pub const PYTH_PUSH_ORACLE_ID: Pubkey =
    solana_sdk::pubkey!("pythWSnswVUd12oZpeFP8e9CVaEqJg25g1Vtc2biRsT");

/// Price-update account of a Pyth feed on a push-oracle shard.
#[must_use]
pub fn pyth_price_feed_account(feed_id: &[u8; 32], shard: u16) -> Pubkey {
    Pubkey::find_program_address(&[&shard.to_le_bytes(), feed_id], &PYTH_PUSH_ORACLE_ID).0
}

#[cfg(test)]
mod tests {
    use super::pyth_price_feed_account;

    #[test]
    fn price_feed_address_is_shard_and_feed_specific() {
        let first = [1; 32];
        let second = [2; 32];
        assert_ne!(
            pyth_price_feed_account(&first, 0),
            pyth_price_feed_account(&first, 1)
        );
        assert_ne!(
            pyth_price_feed_account(&first, 0),
            pyth_price_feed_account(&second, 0)
        );
    }

    #[test]
    fn sol_usd_shard_zero_matches_the_sponsored_feed_account() {
        let mut feed = [0; 32];
        hex::decode_to_slice(
            "ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d",
            &mut feed,
        )
        .unwrap();
        assert_eq!(
            pyth_price_feed_account(&feed, 0).to_string(),
            "7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE"
        );
    }
}
