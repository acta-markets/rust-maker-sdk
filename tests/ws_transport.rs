#![cfg(feature = "ws-client")]

use acta_maker_sdk::{ClientMessage, PreparedClientMessage, WsTransportConfig};

#[test]
fn hft_transport_defaults_are_bounded_and_disable_nagle() {
    let config = WsTransportConfig::default();
    assert!(config.tcp_nodelay);
    assert_eq!(config.write_buffer_size, 0);
    assert!(config.max_write_buffer_size < usize::MAX);
    assert!(config.max_message_size <= 8 * 1024 * 1024);
    assert!(config.max_frame_size <= config.max_message_size);
}

#[test]
fn prepared_message_can_be_reused_without_reserializing() {
    let prepared = PreparedClientMessage::new(&ClientMessage::Ping).unwrap();
    let cloned = prepared.clone();
    assert_eq!(prepared.as_str(), r#"{"type":"Ping"}"#);
    assert_eq!(prepared.as_str(), cloned.as_str());
}
