#[test]
fn paging_frames_round_trip_with_required_fields() {
    let frames: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/paging_wire.json")).unwrap();
    for frame in frames {
        let actual = if frame["type"].as_str().unwrap().starts_with("Get") {
            serde_json::to_value(
                serde_json::from_value::<acta_maker_sdk::ws::types::ClientMessage>(frame.clone())
                    .unwrap(),
            )
            .unwrap()
        } else {
            serde_json::to_value(
                serde_json::from_value::<acta_maker_sdk::ws::types::ServerMessage>(frame.clone())
                    .unwrap(),
            )
            .unwrap()
        };
        assert_eq!(actual, frame);
        let mut missing = frame.clone();
        let field = if frame["type"].as_str().unwrap().starts_with("Get") {
            "cursor"
        } else {
            "next_cursor"
        };
        missing["data"].as_object_mut().unwrap().remove(field);
        let rejected = if field == "cursor" {
            serde_json::from_value::<acta_maker_sdk::ws::types::ClientMessage>(missing).is_err()
        } else {
            serde_json::from_value::<acta_maker_sdk::ws::types::ServerMessage>(missing).is_err()
        };
        assert!(rejected, "{} requires {}", frame["type"], field);
    }
}
#[test]
fn old_unpaged_position_read_is_rejected() {
    let mut frames: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("fixtures/paging_wire.json")).unwrap();
    let _ = frames[0]["data"].as_object_mut().unwrap().remove("cursor");
    assert!(
        serde_json::from_value::<acta_maker_sdk::ws::types::ClientMessage>(frames.remove(0))
            .is_err()
    );
    let _ = frames[0]["data"]
        .as_object_mut()
        .unwrap()
        .remove("next_cursor");
    assert!(
        serde_json::from_value::<acta_maker_sdk::ws::types::ServerMessage>(frames.remove(0))
            .is_err()
    );
}
