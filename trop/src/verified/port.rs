//! Verified port numbers and inclusive port ranges.
//!
//! Production glue: `crate::port::Port` validates and offsets port numbers
//! with the functions here, and `crate::port::PortRange` stores a
//! [`PortSpan`] (whose type invariant guarantees `1 <= min <= max`) and
//! iterates it with a [`PortCursor`].

use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// Specifications
// ---------------------------------------------------------------------------

/// A valid port number: `1..=65535` (port 0 is reserved).
pub open spec fn is_valid_port(value: int) -> bool {
    1 <= value <= 65535
}

/// A valid inclusive port range: both ends valid and `min <= max`.
pub open spec fn is_valid_range(min: int, max: int) -> bool {
    is_valid_port(min) && is_valid_port(max) && min <= max
}

/// The ascending sequence `lo, lo + 1, ..., hi` (empty when `lo > hi`).
pub open spec fn ports_between(lo: int, hi: int) -> Seq<u16> {
    Seq::new(if lo <= hi { (hi - lo + 1) as nat } else { 0 }, |i: int| (lo + i) as u16)
}

// ---------------------------------------------------------------------------
// Port numbers
// ---------------------------------------------------------------------------

/// Returns whether `value` is a valid port number.
pub const fn port_is_valid(value: u16) -> (valid: bool)
    ensures
        valid == is_valid_port(value as int),
{
    value != 0
}

/// Returns whether `value` is a privileged port (`< 1024`).
pub const fn port_is_privileged(value: u16) -> (privileged: bool)
    ensures
        privileged == (value < 1024),
{
    value < 1024
}

/// `port + offset`, or `None` when the sum is not a valid port number
/// (it overflows 65535, or it is 0).
pub const fn port_checked_add(port: u16, offset: u16) -> (result: Option<u16>)
    ensures
        result == (if is_valid_port(port + offset) {
            Some((port + offset) as u16)
        } else {
            None::<u16>
        }),
{
    match port.checked_add(offset) {
        Some(sum) if sum > 0 => Some(sum),
        _ => None,
    }
}

/// `port - offset`, or `None` when the difference is not a valid port number
/// (it underflows, or it is 0).
pub const fn port_checked_sub(port: u16, offset: u16) -> (result: Option<u16>)
    ensures
        result == (if is_valid_port(port - offset) {
            Some((port - offset) as u16)
        } else {
            None::<u16>
        }),
{
    match port.checked_sub(offset) {
        Some(difference) if difference > 0 => Some(difference),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Port ranges
// ---------------------------------------------------------------------------

/// A non-empty inclusive range of valid ports.
///
/// The fields are private and [`PortSpan::new`] is the only constructor, so
/// the type invariant `1 <= min <= max` holds for every `PortSpan` value —
/// including values held by unverified code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortSpan {
    min: u16,
    max: u16,
}

impl PortSpan {
    #[verifier::type_invariant]
    spec fn well_formed(self) -> bool {
        is_valid_range(self.min as int, self.max as int)
    }

    /// The lower bound.
    pub closed spec fn spec_min(self) -> u16 {
        self.min
    }

    /// The upper bound.
    pub closed spec fn spec_max(self) -> u16 {
        self.max
    }

    /// The ports in the span, in ascending order.
    pub open spec fn ports(self) -> Seq<u16> {
        ports_between(self.spec_min() as int, self.spec_max() as int)
    }

    /// Builds the span `min..=max`, or `None` unless it is a valid range.
    pub const fn new(min: u16, max: u16) -> (span: Option<PortSpan>)
        ensures
            span is Some <==> is_valid_range(min as int, max as int),
            span matches Some(s) ==> s.spec_min() == min && s.spec_max() == max,
    {
        if min == 0 || max < min {
            None
        } else {
            Some(PortSpan { min, max })
        }
    }

    /// The lower bound (also exposes the type invariant to callers).
    pub const fn min(self) -> (min: u16)
        ensures
            min == self.spec_min(),
            is_valid_range(self.spec_min() as int, self.spec_max() as int),
    {
        proof {
            use_type_invariant(&self);
        }
        self.min
    }

    /// The upper bound (also exposes the type invariant to callers).
    pub const fn max(self) -> (max: u16)
        ensures
            max == self.spec_max(),
            is_valid_range(self.spec_min() as int, self.spec_max() as int),
    {
        proof {
            use_type_invariant(&self);
        }
        self.max
    }

    /// Whether `port` lies in the span.
    pub const fn contains(self, port: u16) -> (inside: bool)
        ensures
            inside == (self.spec_min() <= port <= self.spec_max()),
            inside == self.ports().contains(port),
    {
        proof {
            use_type_invariant(&self);
            if self.spec_min() <= port <= self.spec_max() {
                let i = port - self.spec_min();
                assert(self.ports()[i] == port);
            }
        }
        self.min <= port && port <= self.max
    }

    /// The number of ports in the span. Never overflows and is never zero.
    pub const fn len(self) -> (count: u16)
        ensures
            count as int == self.spec_max() - self.spec_min() + 1,
            count as int == self.ports().len(),
            count >= 1,
    {
        proof {
            use_type_invariant(&self);
        }
        self.max - self.min + 1
    }

    /// Whether the span has no ports. Always `false`: spans are non-empty.
    pub const fn is_empty(self) -> (empty: bool)
        ensures
            !empty,
            empty == (self.ports().len() == 0),
    {
        proof {
            use_type_invariant(&self);
        }
        self.max < self.min
    }
}

/// An ascending cursor over the ports of a [`PortSpan`].
///
/// Starting from [`PortCursor::new`], successive [`PortCursor::advance`] calls
/// yield exactly `span.ports()` — every port of the span once, in ascending
/// order — and then `None` forever.
#[derive(Clone, Copy, Debug)]
pub struct PortCursor {
    next: u16,
    max: u16,
    exhausted: bool,
}

impl PortCursor {
    #[verifier::type_invariant]
    spec fn well_formed(self) -> bool {
        1 <= self.next <= self.max
    }

    /// The ports this cursor has yet to yield, in order.
    pub closed spec fn remaining(self) -> Seq<u16> {
        if self.exhausted {
            Seq::empty()
        } else {
            ports_between(self.next as int, self.max as int)
        }
    }

    /// A cursor positioned at the start of `span`.
    pub const fn new(span: PortSpan) -> (cursor: PortCursor)
        ensures
            cursor.remaining() == span.ports(),
    {
        PortCursor { next: span.min(), max: span.max(), exhausted: false }
    }

    /// Yields the next port, if any.
    pub fn advance(&mut self) -> (port: Option<u16>)
        ensures
            old(self).remaining().len() == 0 ==> port is None && final(self).remaining() == old(
                self,
            ).remaining(),
            old(self).remaining().len() > 0 ==> port == Some(old(self).remaining()[0])
                && final(self).remaining() == old(self).remaining().drop_first(),
            port matches Some(p) ==> is_valid_port(p as int),
    {
        proof {
            use_type_invariant(&*self);
        }
        if self.exhausted {
            None
        } else {
            let port = self.next;
            if self.next == self.max {
                self.exhausted = true;
            } else {
                self.next += 1;
            }
            proof {
                assert(final(self).remaining() =~= old(self).remaining().drop_first());
            }
            Some(port)
        }
    }

    /// The number of ports this cursor has yet to yield.
    pub fn remaining_len(self) -> (count: usize)
        ensures
            count as int == self.remaining().len(),
    {
        proof {
            use_type_invariant(&self);
        }
        if self.exhausted {
            0
        } else {
            usize::from(self.max - self.next) + 1
        }
    }
}

} // verus!
