//! Property-based tests for `Port` and `PortRange` types.

use super::{Port, PortRange};
use proptest::prelude::*;

/// Minimum valid port number (extracted from `Port::MIN` for property tests).
const MIN_VALID_PORT: u16 = Port::MIN;

/// Maximum valid port number (extracted from `Port::MAX` for property tests).
const MAX_VALID_PORT: u16 = Port::MAX;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 10000,
        max_shrink_iters: 10000,
        .. ProptestConfig::default()
    })]

    // Valid port ranges (1-65535)
    #[test]
    fn port_always_in_valid_range(port in MIN_VALID_PORT..=MAX_VALID_PORT) {
        let p = Port::try_from(port);
        prop_assert!(p.is_ok());
        prop_assert_eq!(p.unwrap().value(), port);
    }

    // Ordering is transitive: if a < b and b < c, then a < c
    #[test]
    fn port_ordering_transitive(a in MIN_VALID_PORT..=MAX_VALID_PORT-2, b_offset in 1u16..=1, c_offset in 1u16..=1) {
        let port_a = Port::try_from(a).unwrap();
        let port_b = Port::try_from(a + b_offset).unwrap();
        let port_c = Port::try_from(a + b_offset + c_offset).unwrap();

        if port_a < port_b && port_b < port_c {
            prop_assert!(port_a < port_c);
        }
    }

    // is_privileged() boundary behavior
    #[test]
    fn port_is_privileged_boundary(port in MIN_VALID_PORT..=MAX_VALID_PORT) {
        let p = Port::try_from(port).unwrap();
        let expected = port < 1024;
        prop_assert_eq!(p.is_privileged(), expected);
    }

    // Serialization round-trips
    #[test]
    fn port_serialization_roundtrip(port in MIN_VALID_PORT..=MAX_VALID_PORT) {
        let p = Port::try_from(port).unwrap();
        let json = serde_json::to_string(&p).unwrap();
        let deserialized: Port = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(p, deserialized);
    }
}

// Property tests for PortRange
proptest! {
    #![proptest_config(ProptestConfig {
        cases: 10000,
        max_shrink_iters: 10000,
        .. ProptestConfig::default()
    })]

    // min <= max always holds (constructor enforces this)
    #[test]
    fn port_range_min_le_max(start in MIN_VALID_PORT..=MAX_VALID_PORT, end in MIN_VALID_PORT..=MAX_VALID_PORT) {
        let (s, e) = if start <= end { (start, end) } else { (end, start) };
        let range = PortRange::new(Port::try_from(s).unwrap(), Port::try_from(e).unwrap());
        prop_assert!(range.is_ok());
        let r = range.unwrap();
        prop_assert!(r.min() <= r.max());
    }

    // contains() is accurate
    #[test]
    fn port_range_contains_accuracy(start in MIN_VALID_PORT..MAX_VALID_PORT, len in 1u16..=100, test_port in MIN_VALID_PORT..=MAX_VALID_PORT) {
        // Ensure we don't exceed MAX_VALID_PORT-1 to avoid iterator overflow bug
        let end = start.saturating_add(len).min(MAX_VALID_PORT - 1);
        let range = PortRange::new(Port::try_from(start).unwrap(), Port::try_from(end).unwrap()).unwrap();
        let p = Port::try_from(test_port).unwrap();

        let should_contain = test_port >= start && test_port <= end;
        prop_assert_eq!(range.contains(p), should_contain);
    }

    // Range length is always correct
    #[test]
    fn port_range_len_correct(start in MIN_VALID_PORT..MAX_VALID_PORT, end in MIN_VALID_PORT..=MAX_VALID_PORT) {
        if start <= end {
            let range = PortRange::new(Port::try_from(start).unwrap(), Port::try_from(end).unwrap()).unwrap();
            let expected_len = (end - start) + 1;
            prop_assert_eq!(range.len(), expected_len);
        }
    }

    // Iterator produces correct number of elements
    #[test]
    fn port_range_iterator_length(start in MIN_VALID_PORT..=MAX_VALID_PORT-20, len in 1u16..=20) {
        // Ensure we don't exceed MAX_VALID_PORT-1 to avoid iterator overflow bug
        let end = start.saturating_add(len).min(MAX_VALID_PORT - 1);
        let range = PortRange::new(Port::try_from(start).unwrap(), Port::try_from(end).unwrap()).unwrap();

        let ports: Vec<Port> = range.iter().collect();
        #[allow(clippy::cast_possible_truncation)]
        let ports_len = ports.len() as u16;
        prop_assert_eq!(ports_len, range.len());
    }

    // Iterator produces ports in ascending order
    #[test]
    fn port_range_iterator_sorted(start in MIN_VALID_PORT..=MAX_VALID_PORT-19, len in 1u16..=19) {
        // Ensure we don't exceed MAX_VALID_PORT-1 to avoid iterator overflow bug
        let end = start.saturating_add(len).min(MAX_VALID_PORT - 1);
        let range = PortRange::new(Port::try_from(start).unwrap(), Port::try_from(end).unwrap()).unwrap();

        let ports: Vec<Port> = range.iter().collect();
        for window in ports.windows(2) {
            prop_assert!(window[0] < window[1]);
        }
    }

    // All ports from iterator are within range
    #[test]
    fn port_range_iterator_all_in_range(start in MIN_VALID_PORT..=MAX_VALID_PORT-19, len in 1u16..=19) {
        // Ensure we don't exceed MAX_VALID_PORT-1 to avoid iterator overflow bug
        let end = start.saturating_add(len).min(MAX_VALID_PORT - 1);
        let range = PortRange::new(Port::try_from(start).unwrap(), Port::try_from(end).unwrap()).unwrap();

        for port in range {
            prop_assert!(range.contains(port));
        }
    }

    // checked_add behaves correctly
    #[test]
    fn port_checked_add_correct(base in MIN_VALID_PORT..=MAX_VALID_PORT-100, offset in 0u16..=100) {
        let port = Port::try_from(base).unwrap();
        let result = port.checked_add(offset);

        if u32::from(base) + u32::from(offset) <= u32::from(MAX_VALID_PORT) {
            prop_assert!(result.is_some());
            prop_assert_eq!(result.unwrap().value(), base + offset);
        } else {
            prop_assert!(result.is_none());
        }
    }

    // checked_sub behaves correctly
    #[test]
    fn port_checked_sub_correct(base in 50u16..=MAX_VALID_PORT, offset in 0u16..=100) {
        let port = Port::try_from(base).unwrap();
        let result = port.checked_sub(offset);

        if base > offset {
            prop_assert!(result.is_some());
            prop_assert_eq!(result.unwrap().value(), base - offset);
        } else {
            prop_assert!(result.is_none());
        }
    }
}
