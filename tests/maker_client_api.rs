#![cfg(feature = "ws-client")]

//! Pins the maker client surfaces.
//!
//! Send commands and the shared lifecycle delegates are generated from macros. Referencing each one by
//! path makes a dropped or renamed entry a compile error rather than a silent
//! removal from a published API.

use acta_maker_sdk::quote::{QuoteBuilder, RfqBinding};
use acta_maker_sdk::ws::maker::{MakerDataClient, MakerQuoteClient};

#[test]
fn every_quote_plane_method_exists() {
    let _ = MakerQuoteClient::spawn;

    let _ = MakerQuoteClient::quote;
    let _ = MakerQuoteClient::try_quote;
    let _ = MakerQuoteClient::replace_quote;
    let _ = MakerQuoteClient::try_replace_quote;
    let _ = MakerQuoteClient::batch_quotes;
    let _ = MakerQuoteClient::try_batch_quotes;
    let _ = MakerQuoteClient::cancel_quote;
    let _ = MakerQuoteClient::try_cancel_quote;
    let _ = MakerQuoteClient::cancel_all_quotes;
    let _ = MakerQuoteClient::try_cancel_all_quotes;
    let _ = MakerQuoteClient::indicative_prices_response;
    let _ = MakerQuoteClient::try_indicative_prices_response;

    let _ = MakerQuoteClient::subscribe;
    let _ = MakerQuoteClient::unsubscribe;
    let _ = MakerQuoteClient::add_mints;
    let _ = MakerQuoteClient::remove_mints;
    let _ = MakerQuoteClient::add_channels;
    let _ = MakerQuoteClient::remove_channels;

    let _ = MakerQuoteClient::subscribe_messages;
    let _ = MakerQuoteClient::subscribe_events;
    let _ = MakerQuoteClient::state;
    let _ = MakerQuoteClient::wait_until_ready;
    let _ = MakerQuoteClient::raw;
    let _ = MakerQuoteClient::into_raw;
    let _ = MakerQuoteClient::close;
}

#[test]
fn every_data_plane_method_exists() {
    let _ = MakerDataClient::spawn;
    let _ = MakerDataClient::request::<acta_maker_sdk::ws::types::GetMarketsMessage>;

    let _ = MakerDataClient::subscribe_messages;
    let _ = MakerDataClient::subscribe_events;
    let _ = MakerDataClient::state;
    let _ = MakerDataClient::wait_until_ready;
    let _ = MakerDataClient::raw;
    let _ = MakerDataClient::into_raw;
    let _ = MakerDataClient::close;

    let _ =
        acta_maker_sdk::ws::maker::MakerResponse::<acta_maker_sdk::ws::types::MarketsData>::payload;
    let _ = acta_maker_sdk::ws::maker::MakerResponse::<
        acta_maker_sdk::ws::types::MarketsData,
    >::into_payload;
}

/// The quote-construction surface, so a rename cannot slip out silently.
#[test]
fn every_quote_construction_entry_point_exists() {
    let _ = RfqBinding::from_broadcast;
    let _ = RfqBinding::rfq_id;
    let _ = RfqBinding::quantity;
    let _ = RfqBinding::position_type;
    let _ = RfqBinding::offered_strikes;
    let _ = RfqBinding::quote;

    let _ = QuoteBuilder::strike;
    let _ = QuoteBuilder::price;
    let _ = QuoteBuilder::valid_until;
    let _ = QuoteBuilder::nonce;
    let _ = QuoteBuilder::preimage_args;
    let _ = QuoteBuilder::sign::<acta_maker_sdk::BytesSigner>;

    let _ = acta_maker_sdk::ws::types::QuoteMessage::into_replacement;
}
