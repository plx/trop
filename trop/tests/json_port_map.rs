//! Differential tests for the verified JSON port-map renderer
//! (`verified::json`, used by `JsonFormatter`).
//!
//! The kernel proves the layout, the escaping round trip and the key order;
//! byte equality with serde_json's pretty printer (which `JsonFormatter`
//! used before) is pinned here.

use std::collections::{BTreeMap, HashMap};

use proptest::prelude::*;
use trop::output::{JsonFormatter, OutputFormatter};
use trop::Port;

/// What `JsonFormatter` produced before it used the kernel.
fn serde_reference(allocations: &HashMap<String, Port>) -> String {
    let map: BTreeMap<String, u16> = allocations
        .iter()
        .map(|(tag, port)| (tag.clone(), port.value()))
        .collect();
    serde_json::to_string_pretty(&map).unwrap()
}

fn render(allocations: &HashMap<String, Port>) -> String {
    JsonFormatter.format(allocations).unwrap()
}

fn port(value: u16) -> Port {
    Port::try_from(value).unwrap()
}

fn assert_matches_serde(allocations: &HashMap<String, Port>) {
    let ours = render(allocations);
    assert_eq!(ours, serde_reference(allocations), "for {allocations:?}");
    let parsed: BTreeMap<String, u16> = serde_json::from_str(&ours).unwrap();
    let expected: BTreeMap<String, u16> = allocations
        .iter()
        .map(|(tag, port)| (tag.clone(), port.value()))
        .collect();
    assert_eq!(parsed, expected);
}

#[test]
fn empty_map_is_braces() {
    let empty = HashMap::new();
    assert_eq!(render(&empty), "{}");
    assert_matches_serde(&empty);
}

#[test]
fn layout_snapshot() {
    let mut allocations = HashMap::new();
    allocations.insert("web".to_string(), port(5000));
    allocations.insert("api".to_string(), port(5001));
    allocations.insert("".to_string(), port(1));
    allocations.insert("z".to_string(), port(65535));
    assert_eq!(
        render(&allocations),
        "{\n  \"\": 1,\n  \"api\": 5001,\n  \"web\": 5000,\n  \"z\": 65535\n}"
    );
    assert_matches_serde(&allocations);
}

#[test]
fn escapes_match_serde() {
    let mut allocations = HashMap::new();
    for (index, tag) in [
        "quote\"d",
        "back\\slash",
        "new\nline",
        "tab\there",
        "cr\rhere",
        "\u{8}backspace",
        "\u{c}formfeed",
        "\u{0}nul",
        "\u{1b}[31mred",
        "\u{1f}unit",
        "del\u{7f}",
        "nel\u{85}",
        "line\u{2028}sep",
        "para\u{2029}sep",
        "bidi\u{202e}",
        "caf\u{e9}",
        "emoji\u{1f600}",
        "slash/ok",
    ]
    .iter()
    .enumerate()
    {
        allocations.insert(
            (*tag).to_string(),
            port(1000 + u16::try_from(index).unwrap()),
        );
    }
    let rendered = render(&allocations);
    assert!(rendered.contains("\"quote\\\"d\""));
    assert!(rendered.contains("\"back\\\\slash\""));
    assert!(rendered.contains("\"\\u0000nul\""));
    assert!(rendered.contains("\"\\u001b[31mred\""));
    assert!(rendered.contains("\"\\u001funit\""));
    assert!(rendered.contains("line\u{2028}sep"));
    assert_matches_serde(&allocations);
}

/// Every one- and two-byte UTF-8 character, every character near the
/// line/paragraph separators, the surrogate gap and the plane boundaries,
/// and a sample of the rest, as a key on its own and inside a key.
#[test]
fn every_char_matches_serde() {
    let chars: Vec<char> = (0..=0x10_FFFFu32)
        .filter(|&v| {
            v < 0x800
                || (0x2000..0x2100).contains(&v)
                || (0xD700..0xE100).contains(&v)
                || (0xFF00..0x1_0100).contains(&v)
                || v >= 0x10_FF00
                || v % 61 == 0
        })
        .filter_map(char::from_u32)
        .collect();
    for (chunk_index, chunk) in chars.chunks(2048).enumerate() {
        let mut allocations = HashMap::new();
        for (offset, &c) in chunk.iter().enumerate() {
            let value = u16::try_from((chunk_index * 2048 + offset) % 65535 + 1).unwrap();
            allocations.insert(c.to_string(), port(value));
            allocations.insert(format!("k{c}v"), port(value));
        }
        assert_eq!(render(&allocations), serde_reference(&allocations));
    }
}

fn special_tag() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            Just('"'),
            Just('\\'),
            Just('/'),
            Just('\u{7f}'),
            Just('\u{2028}'),
            Just('\u{2029}'),
            Just('\u{fffd}'),
            (0u32..0x20).prop_map(|v| char::from_u32(v).unwrap()),
            prop::char::range('a', 'e'),
            any::<char>(),
        ],
        0..8,
    )
    .prop_map(|chars| chars.into_iter().collect())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn random_maps_match_serde(
        entries in prop::collection::vec((special_tag(), 1u16..=65535), 0..12),
    ) {
        let allocations: HashMap<String, Port> =
            entries.into_iter().map(|(tag, value)| (tag, port(value))).collect();
        let ours = render(&allocations);
        prop_assert_eq!(&ours, &serde_reference(&allocations));
        let parsed: BTreeMap<String, u16> = serde_json::from_str(&ours).unwrap();
        prop_assert_eq!(parsed.len(), allocations.len());
        for (tag, port) in &allocations {
            prop_assert_eq!(parsed.get(tag), Some(&port.value()));
        }
    }

    #[test]
    fn arbitrary_strings_match_serde(
        entries in prop::collection::vec((any::<String>(), 1u16..=65535), 0..6),
    ) {
        let allocations: HashMap<String, Port> =
            entries.into_iter().map(|(tag, value)| (tag, port(value))).collect();
        prop_assert_eq!(render(&allocations), serde_reference(&allocations));
    }
}
