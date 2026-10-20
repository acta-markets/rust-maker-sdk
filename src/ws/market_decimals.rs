//! Mint decimals per market, learned from `GetMarketDescriptors`.

use std::collections::HashMap;

use crate::types::Decimals;
use crate::ws::types::{MarketDescriptorInfo, ServerMessage};

/// Underlying and quote mint decimals of one market.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenDecimals {
    pub underlying: Decimals,
    pub quote: Decimals,
}

/// Decimals keyed by `market_pda`, so fills and RFQs (which carry atoms and
/// the market PDA, not decimals) can be priced in human units.
///
/// Feed it every [`ServerMessage`] or call [`insert`](Self::insert) with the
/// `MarketDescriptors` response to `GetMarketDescriptors` on `/maker/data`.
#[derive(Debug, Clone, Default)]
pub struct MarketDecimals {
    by_market: HashMap<String, TokenDecimals>,
}

impl MarketDecimals {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, info: &MarketDescriptorInfo) {
        self.by_market.insert(
            info.market.market_pda.clone(),
            TokenDecimals {
                underlying: info.underlying_decimals,
                quote: info.quote_decimals,
            },
        );
    }

    /// Record decimals from a `MarketDescriptors` response; other messages are ignored.
    pub fn observe(&mut self, message: &ServerMessage) {
        if let ServerMessage::MarketDescriptors(data) = message {
            for info in &data.markets {
                self.insert(info);
            }
        }
    }

    #[must_use]
    pub fn get(&self, market_pda: &str) -> Option<TokenDecimals> {
        self.by_market.get(market_pda).copied()
    }
}

#[cfg(test)]
#[path = "market_decimals_tests.rs"]
mod tests;
