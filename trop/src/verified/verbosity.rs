//! Verified kernel: verbosity levels, their names, CLI/environment
//! resolution, and the logger's severity filter.
//!
//! - `parse_verbosity` accepts exactly the three level names `quiet`,
//!   `normal` and `verbose`, compared without ASCII case, and nothing else
//!   (no trimming, no prefixes); `lemma_parse_name_round_trip` shows every
//!   level's name parses back to it, in any ASCII case.
//! - `resolve_verbosity` is the precedence rule for one flag at a time:
//!   `--verbose` wins, then `--quiet`, then a valid `TROP_LOG_MODE` value,
//!   then `Normal`. An invalid environment value is ignored, never an error.
//! - `resolve_cli_verbosity` is the rule the command line uses for both the
//!   logger and the commands: it is `resolve_verbosity` with `--quiet`
//!   taking precedence over `--verbose` when both are given (a request to
//!   suppress output is never overridden), so either flag overrides
//!   `TROP_LOG_MODE`. `invalid_log_mode` says when `TROP_LOG_MODE` is set to
//!   something that is not a level name, so the CLI can diagnose it; an
//!   empty value counts as unset (`lemma_empty_log_mode_is_unset`).
//! - `should_emit` is the logger's filter: a message is printed exactly when
//!   the level's rank reaches the severity's threshold (`Quiet` prints
//!   nothing, `Normal` errors and warnings, `Verbose` everything); it is
//!   monotone in the level and in the severity.
//!
//! Policy note: `Quiet` suppresses the logger's error messages too, as the
//! production logger always did. This treats logged errors as non-essential
//! output (spec: "`--quiet`: Suppress non-essential output"); a command's
//! fatal error is still reported, because the CLI prints it itself rather
//! than through the logger. Making logged errors essential would set
//! `threshold(Severity::Error)` to 0; that is a maintainer decision.
//!
//! Production glue: `crate::logging` (`LogLevel`, `Logger`,
//! `resolve_log_level`, `init_logger`, `invalid_log_mode`).

use vstd::prelude::*;

use super::text;

verus! {

/// Output verbosity, from least to most verbose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verbosity {
    /// No log messages, errors included (see the policy note above).
    Quiet,
    /// Errors and warnings.
    Normal,
    /// Everything, including informational and debug messages.
    Verbose,
}

/// The position of a level in the order `Quiet < Normal < Verbose`.
pub open spec fn rank(v: Verbosity) -> int {
    match v {
        Verbosity::Quiet => 0,
        Verbosity::Normal => 1,
        Verbosity::Verbose => 2,
    }
}

/// The canonical (lowercase) name of a level: `quiet`, `normal`, `verbose`.
pub open spec fn level_name(v: Verbosity) -> Seq<u8> {
    match v {
        Verbosity::Quiet => seq![b'q', b'u', b'i', b'e', b't'],
        Verbosity::Normal => seq![b'n', b'o', b'r', b'm', b'a', b'l'],
        Verbosity::Verbose => seq![b'v', b'e', b'r', b'b', b'o', b's', b'e'],
    }
}

/// The level named by `s`, compared without ASCII case; `None` for every
/// other byte string.
pub open spec fn parse_level(s: Seq<u8>) -> Option<Verbosity> {
    if text::eq_ignore_ascii_case(s, level_name(Verbosity::Quiet)) {
        Some(Verbosity::Quiet)
    } else if text::eq_ignore_ascii_case(s, level_name(Verbosity::Normal)) {
        Some(Verbosity::Normal)
    } else if text::eq_ignore_ascii_case(s, level_name(Verbosity::Verbose)) {
        Some(Verbosity::Verbose)
    } else {
        None
    }
}

/// `b` is an ASCII letter.
pub open spec fn is_ascii_letter(b: u8) -> bool {
    text::is_ascii_lower_byte(b) || text::is_ascii_upper_byte(b)
}

/// Rank is injective: the order on levels is a total order.
pub proof fn lemma_rank_injective(a: Verbosity, b: Verbosity)
    ensures
        rank(a) == rank(b) <==> a == b,
        0 <= rank(a) <= 2,
{
}

/// Parsing is exact: `s` names `v` (ignoring ASCII case) exactly when it
/// parses to `v`, the three names are pairwise distinct even ignoring case,
/// and anything accepted is 5 to 7 ASCII letters, so empty, padded or
/// otherwise decorated input is rejected.
pub proof fn lemma_parse_level_exact(s: Seq<u8>, v: Verbosity)
    ensures
        parse_level(s) == Some(v) <==> text::eq_ignore_ascii_case(s, level_name(v)),
        parse_level(s) is Some ==> 5 <= s.len() <= 7 && forall|i: int|
            0 <= i < s.len() ==> is_ascii_letter(#[trigger] s[i]),
{
    lemma_names_ci_distinct();
    if parse_level(s) is Some {
        let w = parse_level(s)->Some_0;
        assert forall|i: int| 0 <= i < s.len() implies is_ascii_letter(#[trigger] s[i]) by {
            assert(text::ascii_upper(s[i]) == text::ascii_upper(level_name(w)[i]));
        }
    }
    if text::eq_ignore_ascii_case(s, level_name(v)) {
        if parse_level(s) != Some(v) {
            let w = parse_level(s)->Some_0;
            text::lemma_eq_ignore_ascii_case_equivalence(level_name(v), s, level_name(w));
        }
    }
}

/// No two level names are equal ignoring ASCII case.
proof fn lemma_names_ci_distinct()
    ensures
        forall|a: Verbosity, b: Verbosity|
            text::eq_ignore_ascii_case(#[trigger] level_name(a), #[trigger] level_name(b)) ==> a
                == b,
{
    assert forall|a: Verbosity, b: Verbosity|
        text::eq_ignore_ascii_case(#[trigger] level_name(a), #[trigger] level_name(b)) implies a
        == b by {
        // The names have pairwise different lengths (5, 6 and 7 bytes).
        assert(level_name(a).len() == level_name(b).len());
    }
}

/// Every level's name parses back to it, in lowercase and in uppercase.
pub proof fn lemma_parse_name_round_trip(v: Verbosity)
    ensures
        parse_level(level_name(v)) == Some(v),
        parse_level(text::ascii_upper_seq(level_name(v))) == Some(v),
{
    lemma_parse_level_exact(level_name(v), v);
    text::lemma_eq_ignore_ascii_case_equivalence(level_name(v), level_name(v), level_name(v));
    let up = text::ascii_upper_seq(level_name(v));
    text::lemma_ascii_upper_seq_idempotent(level_name(v));
    text::lemma_ascii_upper_seq_eq_iff(up, level_name(v));
    lemma_parse_level_exact(up, v);
}

/// Parsing ignores ASCII case: strings equal ignoring ASCII case parse to
/// the same result.
pub proof fn lemma_parse_level_case_insensitive(s: Seq<u8>, t: Seq<u8>)
    requires
        text::eq_ignore_ascii_case(s, t),
    ensures
        parse_level(s) == parse_level(t),
{
    text::lemma_eq_ignore_ascii_case_equivalence(s, t, t);
    text::lemma_eq_ignore_ascii_case_equivalence(t, s, s);
    assert forall|v: Verbosity| #![auto] parse_level(s) == Some(v) <==> parse_level(t) == Some(v) by {
        lemma_parse_level_exact(s, v);
        lemma_parse_level_exact(t, v);
        text::lemma_eq_ignore_ascii_case_equivalence(s, t, level_name(v));
        text::lemma_eq_ignore_ascii_case_equivalence(t, s, level_name(v));
    }
    if parse_level(s) is Some {
        assert(parse_level(t) == parse_level(s));
    } else if parse_level(t) is Some {
        assert(parse_level(s) == parse_level(t));
    }
}

/// The single precedence rule: `--verbose`, then `--quiet`, then a valid
/// `TROP_LOG_MODE` value, then `Normal`.
pub open spec fn resolved_level(verbose: bool, quiet: bool, env: Option<Seq<u8>>) -> Verbosity {
    if verbose {
        Verbosity::Verbose
    } else if quiet {
        Verbosity::Quiet
    } else {
        match env {
            Some(e) => match parse_level(e) {
                Some(v) => v,
                None => Verbosity::Normal,
            },
            None => Verbosity::Normal,
        }
    }
}

/// Monotonicity: setting `--verbose` never lowers the level, setting
/// `--quiet` (without `--verbose`) gives the lowest level, and with no flag
/// a valid environment value is used as is.
pub proof fn lemma_resolution_monotone(quiet: bool, env: Option<Seq<u8>>)
    ensures
        rank(resolved_level(true, quiet, env)) >= rank(resolved_level(false, quiet, env)),
        resolved_level(false, true, env) == Verbosity::Quiet,
        forall|v: Verbosity|
            #![auto]
            env == Some(level_name(v)) ==> resolved_level(false, false, env) == v,
{
    assert forall|v: Verbosity| #![auto] env == Some(level_name(v)) implies resolved_level(
        false,
        false,
        env,
    ) == v by {
        lemma_parse_name_round_trip(v);
    }
}

/// The command line's precedence: `--quiet` wins over `--verbose`, then
/// `--verbose`, then a valid `TROP_LOG_MODE` value, then `Normal`. It is
/// `resolved_level` with `--verbose` ignored when `--quiet` is given.
pub open spec fn cli_level(verbose: bool, quiet: bool, env: Option<Seq<u8>>) -> Verbosity {
    resolved_level(verbose && !quiet, quiet, env)
}

/// The command-line rule: either flag overrides the environment (the
/// environment matters only without flags, and then a valid level name is
/// used as is); `--quiet` always gives `Quiet` and `--verbose` without
/// `--quiet` always gives `Verbose`; and the flags never make the result
/// both quiet and verbose (it is one level).
pub proof fn lemma_cli_level(verbose: bool, quiet: bool, e1: Option<Seq<u8>>, e2: Option<Seq<u8>>)
    ensures
        verbose || quiet ==> cli_level(verbose, quiet, e1) == cli_level(verbose, quiet, e2),
        quiet ==> cli_level(verbose, quiet, e1) == Verbosity::Quiet,
        verbose && !quiet ==> cli_level(verbose, quiet, e1) == Verbosity::Verbose,
        !verbose && !quiet ==> cli_level(verbose, quiet, e1) == resolved_level(false, false, e1),
        forall|v: Verbosity|
            #![auto]
            e1 == Some(level_name(v)) ==> cli_level(false, false, e1) == v,
        rank(cli_level(verbose, false, e1)) >= rank(cli_level(false, false, e1)),
{
    lemma_resolution_monotone(false, e1);
}

/// `TROP_LOG_MODE` is set to something that is not a level name. An empty
/// value (`TROP_LOG_MODE=`, the usual way to clear a variable) counts as
/// unset: it is not diagnosed, and `resolved_level` gives `Normal` for it,
/// as for an unset variable (`lemma_empty_log_mode_is_unset`).
pub open spec fn is_invalid_log_mode(env: Option<Seq<u8>>) -> bool {
    env matches Some(e) && e.len() > 0 && parse_level(e) is None
}

/// An empty `TROP_LOG_MODE` behaves exactly like an unset one: it resolves
/// to the same level for every combination of flags, and it is not
/// diagnosed. Every other value that is not a level name is diagnosed.
pub proof fn lemma_empty_log_mode_is_unset(verbose: bool, quiet: bool, e: Seq<u8>)
    ensures
        cli_level(verbose, quiet, Some(Seq::<u8>::empty())) == cli_level(verbose, quiet, None),
        !is_invalid_log_mode(Some(Seq::<u8>::empty())),
        !is_invalid_log_mode(None),
        e.len() > 0 ==> (is_invalid_log_mode(Some(e)) <==> parse_level(e) is None),
{
    // Every accepted name has 5 to 7 bytes.
    lemma_parse_level_exact(Seq::<u8>::empty(), Verbosity::Normal);
}

/// A message severity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// Errors.
    Error,
    /// Warnings.
    Warn,
    /// Informational messages.
    Info,
    /// Debug messages.
    Debug,
}

/// Severities from most to least severe.
pub open spec fn severity_rank(s: Severity) -> int {
    match s {
        Severity::Error => 0,
        Severity::Warn => 1,
        Severity::Info => 2,
        Severity::Debug => 3,
    }
}

/// The lowest level at which a severity is printed.
pub open spec fn threshold(s: Severity) -> int {
    match s {
        Severity::Error | Severity::Warn => 1,
        Severity::Info | Severity::Debug => 2,
    }
}

/// The filter table: `Quiet` prints nothing, `Normal` exactly errors and
/// warnings, `Verbose` everything; and the filter is monotone in the level
/// and in the severity.
pub proof fn lemma_filter_table(l: Verbosity, l2: Verbosity, s: Severity, s2: Severity)
    ensures
        l == Verbosity::Quiet ==> !(rank(l) >= threshold(s)),
        l == Verbosity::Normal ==> (rank(l) >= threshold(s) <==> (s == Severity::Error || s
            == Severity::Warn)),
        l == Verbosity::Verbose ==> rank(l) >= threshold(s),
        rank(l) >= threshold(s) && rank(l) <= rank(l2) ==> rank(l2) >= threshold(s),
        rank(l) >= threshold(s) && severity_rank(s2) <= severity_rank(s) ==> rank(l) >= threshold(
            s2,
        ),
{
}

// ---------------------------------------------------------------------------
// Executable functions
// ---------------------------------------------------------------------------

/// Whether `s` equals the lowercase literal `lit` ignoring ASCII case.
fn matches_name(s: &[u8], lit: &[u8]) -> (r: bool)
    ensures
        r == text::eq_ignore_ascii_case(s@, lit@),
{
    text::bytes_eq_ignore_ascii_case(s, lit)
}

/// Parses a level name (see `parse_level`).
// Verus cannot see the bytes of `b"..."` literals, so the literals here are
// byte-char arrays, which the proof checks against the spec.
#[allow(clippy::byte_char_slices)]
pub fn parse_verbosity(s: &[u8]) -> (r: Option<Verbosity>)
    ensures
        r == parse_level(s@),
{
    let quiet = [b'q', b'u', b'i', b'e', b't'];
    let normal = [b'n', b'o', b'r', b'm', b'a', b'l'];
    let verbose = [b'v', b'e', b'r', b'b', b'o', b's', b'e'];
    proof {
        assert(quiet@ =~= level_name(Verbosity::Quiet));
        assert(normal@ =~= level_name(Verbosity::Normal));
        assert(verbose@ =~= level_name(Verbosity::Verbose));
    }
    if matches_name(s, quiet.as_slice()) {
        Some(Verbosity::Quiet)
    } else if matches_name(s, normal.as_slice()) {
        Some(Verbosity::Normal)
    } else if matches_name(s, verbose.as_slice()) {
        Some(Verbosity::Verbose)
    } else {
        None
    }
}

/// Resolves the effective level from the CLI flags and the raw
/// `TROP_LOG_MODE` value (see `resolved_level`).
pub fn resolve_verbosity(verbose: bool, quiet: bool, env: Option<&[u8]>) -> (v: Verbosity)
    ensures
        v == resolved_level(
            verbose,
            quiet,
            match env {
                Some(e) => Some(e@),
                None => None,
            },
        ),
{
    if verbose {
        Verbosity::Verbose
    } else if quiet {
        Verbosity::Quiet
    } else {
        match env {
            Some(e) => match parse_verbosity(e) {
                Some(v) => v,
                None => Verbosity::Normal,
            },
            None => Verbosity::Normal,
        }
    }
}

/// Resolves the command line's level (see `cli_level`): `--quiet` wins over
/// `--verbose`, either flag wins over `TROP_LOG_MODE`.
pub fn resolve_cli_verbosity(verbose: bool, quiet: bool, env: Option<&[u8]>) -> (v: Verbosity)
    ensures
        v == cli_level(
            verbose,
            quiet,
            match env {
                Some(e) => Some(e@),
                None => None,
            },
        ),
        quiet ==> v == Verbosity::Quiet,
        verbose && !quiet ==> v == Verbosity::Verbose,
{
    resolve_verbosity(verbose && !quiet, quiet, env)
}

/// Decides `is_invalid_log_mode`.
pub fn invalid_log_mode(env: Option<&[u8]>) -> (r: bool)
    ensures
        r == is_invalid_log_mode(
            match env {
                Some(e) => Some(e@),
                None => None,
            },
        ),
{
    match env {
        Some(e) => !e.is_empty() && parse_verbosity(e).is_none(),
        None => false,
    }
}

/// Whether a message of severity `severity` is printed at `level`.
pub const fn should_emit(level: Verbosity, severity: Severity) -> (r: bool)
    ensures
        r == (rank(level) >= threshold(severity)),
{
    match severity {
        Severity::Error | Severity::Warn => !matches!(level, Verbosity::Quiet),
        Severity::Info | Severity::Debug => matches!(level, Verbosity::Verbose),
    }
}

} // verus!
