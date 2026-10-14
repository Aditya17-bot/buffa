//! Edition 2024 feature resolution: field_presence, repeated_field_encoding,
//! enum_type (open/closed), LEGACY_REQUIRED.
//!
//! Gated on has_edition_2024 (requires protoc v32+).

use crate::ed2024::*;
use buffa::Message;

fn round_trip<T: Message + Default + PartialEq + core::fmt::Debug>(msg: &T) -> T {
    T::decode(&mut msg.encode_to_vec().as_slice()).expect("decode")
}

#[test]
fn test_edition_2024_feature_resolution() {
    // Compile-time assertions via type usage: implicit_int is bare i32
    // (field_presence = IMPLICIT override), explicit_int is Option<i32>
    // (edition default is EXPLICIT).
    let msg = EditionTest {
        implicit_int: 42,
        explicit_int: Some(7),
        text: Some("hello".into()),
        packed_ints: vec![1, 2, 3],
        expanded_ints: vec![4, 5, 6],
        sub: buffa::MessageField::some(SubMessage {
            value: Some("inner".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded, msg);
}

#[test]
fn test_edition_2024_implicit_zero_not_encoded() {
    // implicit_int = 0 (default) should NOT be on the wire.
    let msg = EditionTest {
        explicit_int: Some(0),
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    // Field 2 (explicit_int, varint): tag 0x10. No field-1 tag 0x08.
    assert!(!bytes.contains(&0x08), "implicit 0 must not encode");
    assert!(bytes.contains(&0x10), "explicit Some(0) must encode");
}

#[test]
fn test_edition_2024_repeated_encoding_feature() {
    // packed_ints uses file-level PACKED (wire type 2, single tag).
    // expanded_ints uses field-level EXPANDED override (wire type 0, per-element tag).
    let msg = EditionTest {
        packed_ints: vec![1, 2, 3],
        expanded_ints: vec![4, 5, 6],
        ..Default::default()
    };
    let bytes = msg.encode_to_vec();
    // Field 3 packed: tag 0x1A (field 3, wire type 2), appears once.
    assert_eq!(bytes.iter().filter(|&&b| b == 0x1A).count(), 1);
    // Field 4 expanded: tag 0x20 (field 4, wire type 0), appears 3 times.
    assert_eq!(bytes.iter().filter(|&&b| b == 0x20).count(), 3);
    assert_eq!(round_trip(&msg), msg);
}

#[test]
fn test_edition_2024_enum_open() {
    let msg = WithEnums {
        open_status: Some(buffa::EnumValue::Known(OpenStatus::OPEN_ACTIVE)),
        ..Default::default()
    };
    assert_eq!(round_trip(&msg), msg);
    // Unknown value preserved (open enum).
    let unknown = WithEnums {
        open_status: Some(buffa::EnumValue::Unknown(99)),
        ..Default::default()
    };
    assert_eq!(round_trip(&unknown), unknown);
}

#[test]
fn test_edition_2024_enum_closed_no_wrapper() {
    // Per-enum `option features.enum_type = CLOSED` must produce
    // bare `ClosedStatus`, not `EnumValue<ClosedStatus>`. This is a
    // type-level assertion: if closed_status were EnumValue<>, this
    // wouldn't compile.
    let msg = WithEnums {
        closed_status: Some(ClosedStatus::CLOSED_ACTIVE),
        closed_repeated: vec![
            ClosedStatus::CLOSED_UNSPECIFIED,
            ClosedStatus::CLOSED_ACTIVE,
        ],
        closed_map: [("k".into(), ClosedStatus::CLOSED_ACTIVE)]
            .into_iter()
            .collect(),
        nested_closed: Some(enum_container::NestedClosed::NESTED_VALUE),
        ..Default::default()
    };
    let decoded = round_trip(&msg);
    assert_eq!(decoded.closed_status, Some(ClosedStatus::CLOSED_ACTIVE));
    assert_eq!(decoded.closed_repeated.len(), 2);
    assert_eq!(decoded.closed_map["k"], ClosedStatus::CLOSED_ACTIVE);
    assert_eq!(
        decoded.nested_closed,
        Some(enum_container::NestedClosed::NESTED_VALUE)
    );
}

#[test]
fn test_edition_2024_enum_closed_unknown_discarded() {
    // Closed enums: unknown wire values are silently discarded for
    // optional fields (field stays None).
    use buffa::encoding::{encode_varint, Tag, WireType};
    let mut wire = Vec::new();
    // Field 2 (closed_status, varint), value 99 (unknown).
    Tag::new(2, WireType::Varint).encode(&mut wire);
    encode_varint(99, &mut wire);
    let decoded = WithEnums::decode(&mut wire.as_slice()).expect("decode");
    assert_eq!(decoded.closed_status, None, "unknown closed enum discarded");
}

#[test]
fn test_legacy_required_field_presence() {
    // LEGACY_REQUIRED is editions' spelling of proto2 `required`. Must:
    // 1. Produce bare types (not Option<T>)
    // 2. Always encode, even when value equals type default (zero/empty)
    //
    // Previously only LABEL_REQUIRED was checked; LEGACY_REQUIRED fields
    // got proto3-style default suppression (zero values omitted from wire).

    // Type-level: req_int is bare i32, opt_int is Option<i32>.
    let _: i32 = WithLegacyRequired::default().req_int;
    let _: String = WithLegacyRequired::default().req_str;
    let _: Option<i32> = WithLegacyRequired::default().opt_int;

    // Runtime: zero-value required fields must still serialize.
    let msg = WithLegacyRequired {
        req_int: 0,
        req_str: String::new(),
        opt_int: None,
        ..Default::default()
    };
    let wire = msg.encode_to_vec();
    // Zero-value required int32: tag=0x08 + varint(0)=0x00 = 2 bytes
    // Empty required string: tag=0x12 + len=0x00 = 2 bytes
    // opt_int is None → 0 bytes
    assert_eq!(wire.len(), 4, "LEGACY_REQUIRED must always encode");
    assert_eq!(&wire[..], &[0x08, 0x00, 0x12, 0x00]);

    // Round-trip.
    let decoded = round_trip(&msg);
    assert_eq!(decoded.req_int, 0);
    assert_eq!(decoded.req_str, "");
    assert_eq!(decoded.opt_int, None);
}

#[test]
fn test_legacy_required_custom_default() {
    // LEGACY_REQUIRED + [default = X] produces a hand-written impl Default.
    // The is_required_field helper (12fd82d) feeds into both the bare-type
    // decision AND the has_custom detection in generate_custom_default.
    let d = LegacyReqDefault::default();
    assert_eq!(d.req, 42);
    // And always-encodes (even the default value).
    let wire = d.encode_to_vec();
    assert_eq!(&wire[..], &[0x08, 42], "tag + varint(42)");
}
