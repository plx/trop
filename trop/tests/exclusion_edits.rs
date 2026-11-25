//! Tests for the exclusion-list edits behind `trop exclude`,
//! `trop compact-exclusions`, and `trop scan --autoexclude`: the glue around
//! the verified `exclusions` and `config_rewrite` kernels (entry conversion,
//! error mapping, `Port` reconstruction, and the YAML rewrite), checked at
//! the boundaries and against simple set oracles.

use proptest::prelude::*;
use std::cell::RefCell;
use std::collections::BTreeSet;
use trop::config::PortExclusion;
use trop::port::exclusions::{
    add_exclusion_to_list, add_ports_to_list, compact_exclusions, excluded_ports_in,
    first_reserved_port, is_compact, rewrite_excluded_ports, select_write_target, WriteScope,
    WriteTarget,
};
use trop::Port;

fn single(port: u16) -> PortExclusion {
    PortExclusion::Single(port)
}

fn range(start: u16, end: u16) -> PortExclusion {
    PortExclusion::Range { start, end }
}

fn bounds(exclusion: &PortExclusion) -> (u16, u16) {
    match *exclusion {
        PortExclusion::Single(port) => (port, port),
        PortExclusion::Range { start, end } => (start, end),
    }
}

/// The ports a list excludes (oracle).
fn ports_of(list: &[PortExclusion]) -> BTreeSet<u16> {
    list.iter()
        .flat_map(|exclusion| {
            let (lo, hi) = bounds(exclusion);
            lo..=hi
        })
        .collect()
}

fn overlaps(exclusion: &PortExclusion, lo: u16, hi: u16) -> bool {
    let (a, b) = bounds(exclusion);
    a <= hi && lo <= b
}

// ============================================================================
// add_exclusion_to_list
// ============================================================================

#[test]
fn identical_and_covered_entries_are_no_ops() {
    let list = vec![single(8080), range(9000, 9010)];
    for exclusion in [
        single(8080),
        range(8080, 8080),
        range(9000, 9010),
        single(9000),
        single(9010),
        range(9003, 9007),
    ] {
        let (out, changed) = add_exclusion_to_list(&list, &exclusion).unwrap();
        assert!(!changed, "{exclusion:?}");
        assert_eq!(out, list, "{exclusion:?}");
    }
}

#[test]
fn overlapping_entries_merge_into_one_range_at_the_first_position() {
    let list = vec![
        single(1000),
        range(5000, 5005),
        single(2000),
        range(5010, 5020),
        single(3000),
    ];
    let (out, changed) = add_exclusion_to_list(&list, &range(5004, 5012)).unwrap();
    assert!(changed);
    assert_eq!(
        out,
        vec![single(1000), range(5000, 5020), single(2000), single(3000)]
    );
}

#[test]
fn merging_follows_overlaps_through_merged_entries() {
    // `5003` does not overlap `5009..5012`, but lies within the merged block.
    let list = vec![range(5000, 5010), single(5003), single(7000)];
    let (out, changed) = add_exclusion_to_list(&list, &range(5009, 5012)).unwrap();
    assert!(changed);
    assert_eq!(out, vec![range(5000, 5012), single(7000)]);

    // The merged range takes the place of the first entry it absorbs, even
    // when that entry overlaps only another absorbed entry (found by the
    // differential test below), here at the top of the port space.
    let list = vec![single(65532), range(65528, 65532)];
    let (out, changed) = add_exclusion_to_list(&list, &range(65525, 65528)).unwrap();
    assert!(changed);
    assert_eq!(out, vec![range(65525, 65532)]);
}

#[test]
fn adjacent_entries_are_appended_not_merged() {
    let list = vec![range(5000, 5009)];
    let (out, changed) = add_exclusion_to_list(&list, &single(5010)).unwrap();
    assert!(changed);
    assert_eq!(out, vec![range(5000, 5009), single(5010)]);

    let (out, changed) = add_exclusion_to_list(&list, &single(4999)).unwrap();
    assert!(changed);
    assert_eq!(out, vec![range(5000, 5009), single(4999)]);
}

#[test]
fn port_space_boundaries() {
    let (out, changed) = add_exclusion_to_list(&[], &single(1)).unwrap();
    assert!(changed);
    assert_eq!(out, vec![single(1)]);

    let (out, changed) = add_exclusion_to_list(&[range(65530, 65535)], &single(65535)).unwrap();
    assert!(!changed);
    assert_eq!(out, vec![range(65530, 65535)]);

    let (out, changed) = add_exclusion_to_list(&[single(65534)], &single(65535)).unwrap();
    assert!(changed);
    assert_eq!(out, vec![single(65534), single(65535)]);

    let (out, changed) =
        add_exclusion_to_list(&[range(1, 10), single(65535)], &range(5, 65535)).unwrap();
    assert!(changed);
    assert_eq!(out, vec![range(1, 65535)]);
}

#[test]
fn invalid_entries_are_reported_and_nothing_is_added() {
    let error = add_exclusion_to_list(&[single(8080)], &range(9000, 8000)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "validation error for 'exclusion': Invalid range: 9000..8000 (end < start)"
    );
    let error = add_exclusion_to_list(&[single(8080)], &single(0)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "validation error for 'exclusion': Invalid port: 0"
    );
    let error = add_exclusion_to_list(&[single(8080), range(0, 3)], &single(9000)).unwrap_err();
    assert_eq!(
        error.to_string(),
        "validation error for 'excluded_ports[1]': Invalid start port: 0"
    );
}

// ============================================================================
// add_ports_to_list
// ============================================================================

fn ports(values: &[u16]) -> Vec<Port> {
    values.iter().map(|&p| Port::try_from(p).unwrap()).collect()
}

#[test]
fn add_ports_counts_only_newly_excluded_ports() {
    let list = vec![range(8080, 8090)];
    let (out, added) = add_ports_to_list(&list, &ports(&[8085, 9000, 9000, 8090, 1, 65535]));
    assert_eq!(added, 3);
    assert_eq!(
        out,
        vec![range(8080, 8090), single(9000), single(1), single(65535)]
    );

    let (again, added) = add_ports_to_list(&out, &ports(&[9000, 1, 65535]));
    assert_eq!(added, 0);
    assert_eq!(again, out);

    let (empty, added) = add_ports_to_list(&[], &[]);
    assert_eq!(added, 0);
    assert!(empty.is_empty());
}

// ============================================================================
// is_compact
// ============================================================================

#[test]
fn is_compact_is_the_canonical_form() {
    assert!(is_compact(&[]));
    assert!(is_compact(&[single(1), range(3, 65535)]));
    // Minimal size, but not canonical.
    assert!(!is_compact(&[
        single(9000),
        single(8000),
        range(7000, 7000)
    ]));
    assert!(!is_compact(&[range(7000, 7000)]));
    // Adjacent and overlapping entries.
    assert!(!is_compact(&[single(7000), single(7001)]));
    assert!(!is_compact(&[range(7000, 7005), range(7005, 7010)]));
    // Invalid entries are never compact.
    assert!(!is_compact(&[single(0)]));
    assert!(!is_compact(&[range(9000, 8000)]));
}

// ============================================================================
// first_reserved_port
// ============================================================================

#[test]
fn first_reserved_port_scans_ascending_and_stops() {
    let log = RefCell::new(Vec::new());
    let found = first_reserved_port(&range(8080, 8090), |port| {
        log.borrow_mut().push(port.value());
        Ok::<_, ()>(port.value() >= 8083)
    })
    .unwrap();
    assert_eq!(found.map(Port::value), Some(8083));
    assert_eq!(*log.borrow(), vec![8080, 8081, 8082, 8083]);
}

#[test]
fn first_reserved_port_propagates_errors() {
    let log = RefCell::new(Vec::new());
    let result = first_reserved_port(&range(8080, 8090), |port| {
        log.borrow_mut().push(port.value());
        if port.value() == 8082 {
            Err("database is locked")
        } else {
            Ok(false)
        }
    });
    assert_eq!(result, Err("database is locked"));
    assert_eq!(*log.borrow(), vec![8080, 8081, 8082]);
}

#[test]
fn first_reserved_port_boundaries() {
    // Ends at 65535 without overflowing; every port is checked once.
    let log = RefCell::new(Vec::new());
    let found = first_reserved_port(&range(65533, 65535), |port| {
        log.borrow_mut().push(port.value());
        Ok::<_, ()>(false)
    })
    .unwrap();
    assert_eq!(found, None);
    assert_eq!(*log.borrow(), vec![65533, 65534, 65535]);

    // Port 0 is never consulted (and reversed ranges name no ports).
    let log = RefCell::new(Vec::new());
    let found = first_reserved_port(&range(0, 2), |port| {
        log.borrow_mut().push(port.value());
        Ok::<_, ()>(false)
    })
    .unwrap();
    assert_eq!(found, None);
    assert_eq!(*log.borrow(), vec![1, 2]);
    assert_eq!(
        first_reserved_port(&single(0), |_| Err::<bool, _>(())),
        Ok(None)
    );
    assert_eq!(
        first_reserved_port(&range(9000, 8000), |_| Err::<bool, _>(())),
        Ok(None)
    );
}

// ============================================================================
// rewrite_excluded_ports / excluded_ports_in
// ============================================================================

#[test]
fn rewrite_replaces_in_place_and_keeps_other_keys() {
    let document = "project: demo\nexcluded_ports:\n  - \"5000..5002\"\nreservations: null\nports:\n  min: 5000\n  max: 7000\n";
    let text = rewrite_excluded_ports(document, &[single(6000)]).unwrap();
    assert_eq!(
        text,
        "project: demo\nexcluded_ports:\n- 6000\nreservations: null\nports:\n  min: 5000\n  max: 7000\n"
    );
}

/// Regression: rewrites serialized the whole configuration, turning every
/// absent key into an explicit `null` (`reservations: null` clears the
/// inherited group).
#[test]
fn rewrite_never_introduces_null_keys() {
    let text = rewrite_excluded_ports("excluded_ports:\n  - 6099\n", &[single(6099), single(6098)])
        .unwrap();
    assert_eq!(text, "excluded_ports:\n- 6099\n- 6098\n");

    let text = rewrite_excluded_ports("project: demo\n", &[range(8080, 8090)]).unwrap();
    assert_eq!(
        text,
        "project: demo\nexcluded_ports:\n- start: 8080\n  end: 8090\n"
    );
}

#[test]
fn rewrite_of_empty_and_comment_only_documents() {
    for document in ["", "\n", "# only a comment\n"] {
        assert_eq!(
            rewrite_excluded_ports(document, &[single(1)]).unwrap(),
            "excluded_ports:\n- 1\n",
            "{document:?}"
        );
        assert_eq!(excluded_ports_in(document).unwrap(), None, "{document:?}");
    }
}

#[test]
fn rewrite_rejects_documents_that_are_not_mappings() {
    assert!(rewrite_excluded_ports("- 1\n- 2\n", &[single(1)]).is_err());
    assert!(rewrite_excluded_ports("just text", &[single(1)]).is_err());
    assert!(rewrite_excluded_ports("key: [unclosed", &[single(1)]).is_err());
}

#[test]
fn rewritten_lists_read_back_unchanged() {
    let list = vec![single(1), range(5000, 5010), single(65535)];
    let text = rewrite_excluded_ports("project: demo\nexcluded_ports: null\n", &list).unwrap();
    assert_eq!(excluded_ports_in(&text).unwrap(), Some(list));
}

/// Regression (found in review): the configuration reader ignores YAML tags
/// on keys, so `!tag excluded_ports` is the exclusion list. The rewrite used
/// to compare keys as YAML values, left the tagged key in place and appended
/// a second `excluded_ports`, which made the file unreadable.
#[test]
fn rewrite_replaces_a_tagged_excluded_ports_key() {
    let document = "project: demo\n!foo excluded_ports: [1]\ncleanup: null\n";
    assert_eq!(excluded_ports_in(document).unwrap(), Some(vec![single(1)]));
    let text = rewrite_excluded_ports(document, &[single(1), single(8080)]).unwrap();
    assert_eq!(
        text,
        "project: demo\nexcluded_ports:\n- 1\n- 8080\ncleanup: null\n"
    );
    assert_eq!(
        excluded_ports_in(&text).unwrap(),
        Some(vec![single(1), single(8080)])
    );
}

/// A tagged document reads like an untagged one; the rewrite keeps its tag.
#[test]
fn rewrite_keeps_the_tag_of_a_tagged_document() {
    let document = "--- !foo\nproject: demo\n";
    assert_eq!(excluded_ports_in(document).unwrap(), None);
    let text = rewrite_excluded_ports(document, &[single(8080)]).unwrap();
    assert_eq!(excluded_ports_in(&text).unwrap(), Some(vec![single(8080)]));
    let value: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    let serde_yaml::Value::Tagged(tagged) = value else {
        panic!("the tag was dropped: {text:?}");
    };
    assert_eq!(tagged.tag, serde_yaml::value::Tag::new("foo"));
    assert_eq!(
        tagged.value,
        serde_yaml::from_str::<serde_yaml::Value>("project: demo\nexcluded_ports: [8080]\n")
            .unwrap()
    );
}

/// A rewrite that would change how the loader reads any other key is
/// refused: here the unquoted project name `0x10` is a string to the
/// configuration reader but the number 16 to a YAML value, and would be
/// re-emitted as `16`.
#[test]
fn rewrite_refuses_to_change_how_other_keys_read() {
    let document = "project: 0x10\n";
    assert_eq!(excluded_ports_in(document).unwrap(), None);
    let error = rewrite_excluded_ports(document, &[single(8080)]).unwrap_err();
    assert!(
        error.to_string().contains("only excluded_ports replaced"),
        "{error}"
    );
    // Quoted, it is a string either way and the rewrite goes through.
    assert_eq!(
        rewrite_excluded_ports("project: '0x10'\n", &[single(8080)]).unwrap(),
        "project: '0x10'\nexcluded_ports:\n- 8080\n"
    );
}

/// Documents the loader would not read are refused, never rewritten.
#[test]
fn rewrite_refuses_documents_that_are_not_configurations() {
    for document in [
        "unknown_key: 1\n",
        "excluded_ports: [1]\n!foo excluded_ports: [2]\n",
        "project: [1, 2]\n",
    ] {
        assert!(
            rewrite_excluded_ports(document, &[single(8080)]).is_err(),
            "{document:?}"
        );
    }
}

/// Regression: `excluded_ports_in` and the rewrite's loader-style reader
/// read a document whose content is an explicit null (`null`, `~`, a tagged
/// null) as the default configuration, while `ConfigLoader` rejects it. They
/// now read every document exactly as the loader does: documents with no
/// content are the default configuration, explicit nulls are errors.
#[test]
fn excluded_ports_in_reads_documents_exactly_as_the_loader_does() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trop.yaml");
    let documents = [
        "",
        "\n",
        "# only a comment\n",
        "---\n",
        "--- # a comment\n",
        "---\n...\n",
        "null",
        "null\n",
        "~",
        "~\n",
        "Null\n",
        "NULL\n",
        "--- null\n",
        "!foo null\n",
        "--- !foo\n",
        "!!null ''\n",
        "''\n",
        "project: demo\n",
        "excluded_ports: [1]\n",
        "excluded_ports: null\n",
        "--- !foo\nexcluded_ports: [2]\n",
        "unknown_key: 1\n",
        "- 1\n",
    ];
    for document in documents {
        std::fs::write(&path, document).unwrap();
        let loaded = trop::config::ConfigLoader::load_file(&path);
        let read = excluded_ports_in(document);
        match (&loaded, &read) {
            (Ok(config), Ok(excluded)) => {
                assert_eq!(&config.excluded_ports, excluded, "{document:?}");
            }
            (Err(_), Err(_)) => {}
            _ => panic!("{document:?}: loader {loaded:?}, excluded_ports_in {read:?}"),
        }
        // The rewrite reads the original the same way: it refuses exactly
        // the documents the loader rejects.
        assert_eq!(
            rewrite_excluded_ports(document, &[single(8080)]).is_ok(),
            loaded.is_ok(),
            "{document:?}"
        );
    }
    for document in ["null", "~", "!foo null\n"] {
        assert!(excluded_ports_in(document).is_err(), "{document:?}");
        assert!(
            rewrite_excluded_ports(document, &[single(8080)]).is_err(),
            "{document:?}"
        );
    }
    for document in ["", "# only a comment\n", "---\n"] {
        assert_eq!(excluded_ports_in(document).unwrap(), None, "{document:?}");
    }
}

#[test]
fn excluded_ports_in_validates_the_configuration_shape() {
    assert!(excluded_ports_in("unknown_key: 1\n").is_err());
    assert_eq!(
        excluded_ports_in("excluded_ports:\n  - \"5000..5002\"\n  - 7\n").unwrap(),
        Some(vec![range(5000, 5002), single(7)])
    );
    assert_eq!(excluded_ports_in("project: demo\n").unwrap(), None);
    // One message: `serde_yaml`'s description, under the field
    // `configuration`.
    match excluded_ports_in("bogus: 1\n") {
        Err(trop::Error::Validation { field, message }) => {
            assert_eq!(field, "configuration");
            assert!(message.starts_with("unknown field `bogus`"), "{message}");
        }
        other => panic!("expected a validation error, got {other:?}"),
    }
}

// ============================================================================
// select_write_target
// ============================================================================

#[test]
fn write_target_selection() {
    for local in [false, true] {
        for project in [false, true] {
            assert_eq!(
                select_write_target(WriteScope::Global, local, project),
                Some(WriteTarget::Global)
            );
            let loaded = if local {
                Some(WriteTarget::Local)
            } else if project {
                Some(WriteTarget::Project)
            } else {
                None
            };
            assert_eq!(
                select_write_target(WriteScope::Project, local, project),
                loaded
            );
            assert_eq!(
                select_write_target(WriteScope::ProjectOrGlobal, local, project),
                loaded.or(Some(WriteTarget::Global))
            );
        }
    }
}

// ============================================================================
// Differential tests against set oracles
// ============================================================================

fn arb_entry() -> impl Strategy<Value = PortExclusion> {
    prop_oneof![
        (1u16..=64).prop_map(single),
        (1u16..=64, 0u16..6).prop_map(|(start, len)| range(start, start.saturating_add(len))),
    ]
}

fn arb_edge_entry() -> impl Strategy<Value = PortExclusion> {
    prop_oneof![
        (65530u16..=65535).prop_map(single),
        (65525u16..=65535, 0u16..12)
            .prop_map(|(start, len)| range(start, start.saturating_add(len))),
        arb_entry(),
    ]
}

proptest! {
    /// `trop exclude`: union semantics, no-op exactly when one entry covers
    /// the new one, merge locality, and the changed flag.
    #[test]
    fn add_exclusion_matches_the_set_oracle(
        list in prop::collection::vec(arb_edge_entry(), 0..8),
        new in arb_edge_entry(),
    ) {
        let (out, changed) = add_exclusion_to_list(&list, &new).unwrap();
        let mut expected = ports_of(&list);
        expected.extend(ports_of(std::slice::from_ref(&new)));
        prop_assert_eq!(ports_of(&out), expected);
        prop_assert_eq!(changed, out != list);

        let (nlo, nhi) = bounds(&new);
        let covered = list.iter().any(|e| { let (a, b) = bounds(e); a <= nlo && nhi <= b });
        prop_assert_eq!(!changed, covered);
        if changed {
            // Oracle: grow the block from `new` by every overlapping entry
            // until nothing overlapping it sticks out.
            let (mut lo, mut hi) = (nlo, nhi);
            loop {
                let (old_lo, old_hi) = (lo, hi);
                for entry in &list {
                    if overlaps(entry, lo, hi) {
                        let (a, b) = bounds(entry);
                        lo = lo.min(a);
                        hi = hi.max(b);
                    }
                }
                if (lo, hi) == (old_lo, old_hi) {
                    break;
                }
            }
            let expected = match list.iter().position(|e| overlaps(e, lo, hi)) {
                None => {
                    let mut appended = list.clone();
                    appended.push(new.clone());
                    appended
                }
                Some(first) => {
                    let mut merged = list[..first].to_vec();
                    merged.push(range(lo, hi));
                    merged.extend(list[first + 1..].iter().filter(|e| !overlaps(e, lo, hi)).cloned());
                    merged
                }
            };
            prop_assert_eq!(&out, &expected);
        }
    }

    /// `trop scan --autoexclude`: coverage and the newly-excluded count.
    #[test]
    fn add_ports_matches_the_set_oracle(
        list in prop::collection::vec(arb_edge_entry(), 0..6),
        new in prop::collection::vec(prop_oneof![1u16..=70, 65530u16..=65535], 0..10),
    ) {
        let before = ports_of(&list);
        let (out, added) = add_ports_to_list(&list, &ports(&new));
        let mut expected = before.clone();
        expected.extend(new.iter().copied());
        prop_assert_eq!(ports_of(&out), expected);
        let fresh: BTreeSet<u16> = new.iter().copied().filter(|p| !before.contains(p)).collect();
        prop_assert_eq!(added, fresh.len());
        prop_assert_eq!(out.len(), list.len() + added);
        prop_assert_eq!(&out[..list.len()], &list[..]);
        // The appended singles follow the first occurrence of each newly
        // excluded port.
        let mut seen = BTreeSet::new();
        let appended: Vec<PortExclusion> = new
            .iter()
            .copied()
            .filter(|p| !before.contains(p) && seen.insert(*p))
            .map(single)
            .collect();
        prop_assert_eq!(&out[list.len()..], &appended[..]);
    }

    /// `is_compact` holds exactly when compaction returns the list unchanged.
    #[test]
    fn is_compact_agrees_with_compaction(
        list in prop::collection::vec(arb_edge_entry(), 0..6),
    ) {
        let compacted = compact_exclusions(&list).unwrap();
        prop_assert_eq!(is_compact(&list), compacted == list);
        prop_assert!(is_compact(&compacted));
        prop_assert_eq!(ports_of(&compacted), ports_of(&list));
    }

    /// The rewrite changes only `excluded_ports`, for any combination of
    /// absent, null, and present keys.
    #[test]
    fn rewrite_preserves_every_other_key(
        project in prop_oneof![Just(None), Just(Some("null")), Just(Some("demo"))],
        reservations in prop_oneof![Just(None), Just(Some("null"))],
        excluded in prop_oneof![Just(None), Just(Some("null")), Just(Some("[1, \"3..4\"]"))],
        list in prop::collection::vec(arb_entry(), 0..4),
    ) {
        let mut document = String::new();
        for (key, value) in [("project", project), ("excluded_ports", excluded), ("reservations", reservations)] {
            if let Some(value) = value {
                document.push_str(&format!("{key}: {value}\n"));
            }
        }
        let before: serde_yaml::Mapping = serde_yaml::from_str::<Option<serde_yaml::Mapping>>(&document)
            .unwrap().unwrap_or_default();
        let text = rewrite_excluded_ports(&document, &list).unwrap();
        let after: serde_yaml::Mapping = serde_yaml::from_str(&text).unwrap();
        let key = serde_yaml::Value::from("excluded_ports");
        for (k, v) in &before {
            if *k != key {
                prop_assert_eq!(after.get(k), Some(v));
            }
        }
        for k in after.keys() {
            prop_assert!(*k == key || before.contains_key(k));
        }
        prop_assert_eq!(after.get(&key), Some(&serde_yaml::to_value(&list).unwrap()));
        prop_assert_eq!(excluded_ports_in(&text).unwrap(), Some(list));
    }
}
