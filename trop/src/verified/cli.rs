//! Verified kernel: the command-line surface's decisions.
//!
//! - Exit codes (`exit_code_for`, `process_exit_code`): the specification's
//!   table (0 success, 1 semantic failure, 2 lock timeout, 3 no data
//!   directory, 4 and up other errors) for every way the process can end,
//!   including argument-parsing (usage) errors, which must not reuse the
//!   timeout code.
//! - `is_lock_contention_code`: exactly the `SQLITE_BUSY` and `SQLITE_LOCKED`
//!   result-code families (any extended code whose low byte is 5 or 6) are
//!   lock contention.
//! - `select_data_dir`: the default data directory is the first usable
//!   source of `TROP_DATA_DIR` (explicit), `HOME/.trop` and
//!   `USERPROFILE/.trop`; a home variable is usable only when it is set,
//!   non-empty and absolute, so the default never depends on the working
//!   directory.
//! - `plan_init`: `trop init`'s steps, shared by the live run and the dry
//!   run, specified by the state they produce, what they may never destroy,
//!   and minimality.
//! - `assertion_outcome` / `prints_reservation_port`: the `assert-*`
//!   commands' verdicts and the truthfulness of their failure messages.
//! - `cleanup_switches`, `metadata_intent`, `occupancy_layer`,
//!   `scan_probes`: flag derivations and the occupancy layers (`scan`
//!   ignores a configured global `skip`).
//! - `identifier_filter_matches`: `list --filter-project/--filter-tag`
//!   compare canonical (trimmed) identifiers.
//! - `parse_port_arg`: port arguments, with exactly `u32::from_str` syntax
//!   and the reserve command's error precedence.
//!
//! Production glue: `trop::cli_support`, `crate::database::config`,
//! `crate::error`, `crate::operations::init`, and the `trop-cli` crate.

use vstd::prelude::*;

verus! {

// ===========================================================================
// Exit codes
// ===========================================================================

/// What kind of failure ended a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorClass {
    /// The command ran correctly but its answer is negative (`assert-*`).
    Semantic,
    /// A sticky project or task would change without permission.
    StickyField,
    /// A path is unrelated to the working directory without permission.
    PathRelationship,
    /// Waiting for the database lock timed out.
    LockTimeout,
    /// No data directory was found or could be determined.
    NoDataDir,
    /// The command line is invalid.
    InvalidArguments,
    /// An I/O error.
    Io,
    /// Any other library error.
    OtherLibrary,
    /// A configuration error.
    Config,
}

/// How the process ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Termination {
    /// The command succeeded.
    Success,
    /// `--help` or `--version` was printed.
    Informational,
    /// The arguments could not be parsed (a usage error).
    Usage,
    /// The command failed.
    Failure(ErrorClass),
}

/// The classes the specification calls "semantic errors": the tool ran
/// correctly but got a negative result. A refused sticky-field change and a
/// refused unrelated path are negative answers to "may I change this?".
pub open spec fn is_semantic(c: ErrorClass) -> bool {
    c == ErrorClass::Semantic || c == ErrorClass::StickyField || c == ErrorClass::PathRelationship
}

/// The classes that are "other errors" in the specification (code 4+).
pub open spec fn is_other_error(c: ErrorClass) -> bool {
    !is_semantic(c) && c != ErrorClass::LockTimeout && c != ErrorClass::NoDataDir
}

/// The exit-code table for failed commands.
pub open spec fn exit_code_of(c: ErrorClass) -> i32 {
    match c {
        ErrorClass::Semantic | ErrorClass::StickyField | ErrorClass::PathRelationship => 1,
        ErrorClass::LockTimeout => 2,
        ErrorClass::NoDataDir => 3,
        ErrorClass::InvalidArguments => 4,
        ErrorClass::Io => 5,
        ErrorClass::OtherLibrary => 6,
        ErrorClass::Config => 7,
    }
}

/// The exit code of a failed command.
///
/// The specification fixes 1 (semantic), 2 (timeout) and 3 (no data
/// directory); the remaining classes get distinct codes from 4 up, so the
/// code identifies the class of every non-semantic failure.
#[must_use]
pub const fn exit_code_for(c: ErrorClass) -> (r: i32)
    ensures
        r == exit_code_of(c),
        1 <= r <= 7,
        r == 1 <==> is_semantic(c),
        r == 2 <==> c == ErrorClass::LockTimeout,
        r == 3 <==> c == ErrorClass::NoDataDir,
        r >= 4 <==> is_other_error(c),
        c == ErrorClass::InvalidArguments ==> r == 4,
{
    match c {
        ErrorClass::Semantic | ErrorClass::StickyField | ErrorClass::PathRelationship => 1,
        ErrorClass::LockTimeout => 2,
        ErrorClass::NoDataDir => 3,
        ErrorClass::InvalidArguments => 4,
        ErrorClass::Io => 5,
        ErrorClass::OtherLibrary => 6,
        ErrorClass::Config => 7,
    }
}

/// Exit codes from 2 up identify the failure class: two non-semantic classes
/// with the same code are the same class.
pub proof fn lemma_exit_codes_identify_class(a: ErrorClass, b: ErrorClass)
    requires
        !is_semantic(a),
        !is_semantic(b),
        exit_code_of(a) == exit_code_of(b),
    ensures
        a == b,
{
}

/// The exit status of the whole process.
///
/// Success and the informational `--help`/`--version` output exit 0; a usage
/// error is an invalid-arguments failure (4); a failed command exits with
/// its class's code. So 0 means success, and 2 means a lock timeout and
/// nothing else.
#[must_use]
pub const fn process_exit_code(t: Termination) -> (r: i32)
    ensures
        0 <= r <= 7,
        r == 0 <==> (t == Termination::Success || t == Termination::Informational),
        r == 1 <==> (t matches Termination::Failure(c) && is_semantic(c)),
        r == 2 <==> t == Termination::Failure(ErrorClass::LockTimeout),
        r == 3 <==> t == Termination::Failure(ErrorClass::NoDataDir),
        t == Termination::Usage ==> r == exit_code_of(ErrorClass::InvalidArguments),
        t matches Termination::Failure(c) ==> r == exit_code_of(c),
{
    match t {
        Termination::Success | Termination::Informational => 0,
        Termination::Usage => exit_code_for(ErrorClass::InvalidArguments),
        Termination::Failure(c) => exit_code_for(c),
    }
}

// ===========================================================================
// SQLite lock contention
// ===========================================================================

/// The primary result code of an (extended) SQLite result code: its low
/// eight bits.
pub open spec fn primary_code(extended: i32) -> i32 {
    extended & 0xff
}

/// `SQLITE_BUSY` (5) or `SQLITE_LOCKED` (6), including every extended code
/// of those families.
pub open spec fn is_lock_contention(extended: i32) -> bool {
    primary_code(extended) == 5 || primary_code(extended) == 6
}

/// Whether an SQLite failure is lock contention (`SQLITE_BUSY` or
/// `SQLITE_LOCKED`, with any extended code).
#[must_use]
pub fn is_lock_contention_code(extended: i32) -> (r: bool)
    ensures
        r == is_lock_contention(extended),
{
    let primary = extended & 0xff;
    primary == 5 || primary == 6
}

/// The families are exactly the residues 5 and 6 modulo 256: for a
/// non-negative code, the low byte is the code modulo 256.
pub proof fn lemma_lock_contention_families(extended: i32)
    requires
        extended >= 0,
    ensures
        is_lock_contention(extended) <==> (extended % 256 == 5 || extended % 256 == 6),
{
    assert((extended & 0xff) == extended % 256) by (bit_vector)
        requires
            extended >= 0,
    ;
}

/// Named codes: `SQLITE_BUSY`, `SQLITE_LOCKED` and their extended codes
/// (`BUSY_RECOVERY` 261, `BUSY_SNAPSHOT` 517, `BUSY_TIMEOUT` 773,
/// `LOCKED_SHAREDCACHE` 262, `LOCKED_VTAB` 518) are lock contention;
/// `SQLITE_OK`, `ERROR`, `CORRUPT`, `CANTOPEN`, `NOTADB` and `IOERR_LOCK`
/// (3850) are not.
pub proof fn lemma_lock_contention_examples()
    ensures
        is_lock_contention(5),
        is_lock_contention(6),
        is_lock_contention(261),
        is_lock_contention(517),
        is_lock_contention(773),
        is_lock_contention(262),
        is_lock_contention(518),
        !is_lock_contention(0),
        !is_lock_contention(1),
        !is_lock_contention(11),
        !is_lock_contention(14),
        !is_lock_contention(26),
        !is_lock_contention(3850),
{
    assert((5i32 & 0xff) == 5) by (bit_vector);
    assert((6i32 & 0xff) == 6) by (bit_vector);
    assert((261i32 & 0xff) == 5) by (bit_vector);
    assert((517i32 & 0xff) == 5) by (bit_vector);
    assert((773i32 & 0xff) == 5) by (bit_vector);
    assert((262i32 & 0xff) == 6) by (bit_vector);
    assert((518i32 & 0xff) == 6) by (bit_vector);
    assert((0i32 & 0xff) == 0) by (bit_vector);
    assert((1i32 & 0xff) == 1) by (bit_vector);
    assert((11i32 & 0xff) == 11) by (bit_vector);
    assert((14i32 & 0xff) == 14) by (bit_vector);
    assert((26i32 & 0xff) == 26) by (bit_vector);
    assert((3850i32 & 0xff) == 10) by (bit_vector);
}

// ===========================================================================
// Data-directory selection
// ===========================================================================

/// Facts about a home-directory variable (`HOME` or `USERPROFILE`) that is
/// set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HomeFacts {
    /// The value is not empty.
    pub nonempty: bool,
    /// The value is an absolute path.
    pub absolute: bool,
}

/// Where the default data directory comes from. (An explicit `--data-dir`
/// is used as given and takes precedence over every source here; that
/// choice is made by the command line before the default is consulted.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataDirSource {
    /// `TROP_DATA_DIR`, used as given.
    TropDataDir,
    /// `HOME/.trop`.
    Home,
    /// `USERPROFILE/.trop`.
    UserProfile,
}

/// A home-directory variable can anchor the default data directory: it is
/// set, non-empty and absolute. (An empty or relative value would put the
/// database under the current directory, so different working directories
/// would silently use different databases.)
pub open spec fn usable_home(h: Option<HomeFacts>) -> bool {
    h matches Some(f) && f.nonempty && f.absolute
}

/// The sources in priority order.
pub open spec fn source_rank(s: DataDirSource) -> int {
    match s {
        DataDirSource::TropDataDir => 0,
        DataDirSource::Home => 1,
        DataDirSource::UserProfile => 2,
    }
}

/// The source of rank `i` can be used. `TROP_DATA_DIR` is an explicit
/// setting and is used as given whenever it is set.
pub open spec fn candidate_usable(
    trop_data_dir: bool,
    home: Option<HomeFacts>,
    userprofile: Option<HomeFacts>,
    i: int,
) -> bool {
    if i == 0 {
        trop_data_dir
    } else if i == 1 {
        usable_home(home)
    } else if i == 2 {
        usable_home(userprofile)
    } else {
        false
    }
}

/// Decides `usable_home`.
fn home_is_usable(h: Option<HomeFacts>) -> (u: bool)
    ensures
        u == usable_home(h),
{
    match h {
        Some(f) => f.nonempty && f.absolute,
        None => false,
    }
}

/// Selects the default data directory's source: the first usable source in
/// the order `TROP_DATA_DIR`, `HOME`, `USERPROFILE`, or `None` when none is
/// usable (the "no data directory" error, exit 3, never a panic and never a
/// directory relative to the working directory).
#[must_use]
pub fn select_data_dir(
    trop_data_dir: bool,
    home: Option<HomeFacts>,
    userprofile: Option<HomeFacts>,
) -> (r: Option<DataDirSource>)
    ensures
        r matches Some(s) ==> candidate_usable(trop_data_dir, home, userprofile, source_rank(s))
            && forall|j: int|
            0 <= j < source_rank(s) ==> !candidate_usable(trop_data_dir, home, userprofile, j),
        r is None <==> forall|j: int|
            0 <= j < 3 ==> !candidate_usable(trop_data_dir, home, userprofile, j),
{
    let r = if trop_data_dir {
        Some(DataDirSource::TropDataDir)
    } else if home_is_usable(home) {
        Some(DataDirSource::Home)
    } else if home_is_usable(userprofile) {
        Some(DataDirSource::UserProfile)
    } else {
        None
    };
    proof {
        if let Some(s) = r {
            // The selected source witnesses that some source is usable.
            assert(candidate_usable(trop_data_dir, home, userprofile, source_rank(s)));
        }
    }
    r
}

/// Consequences of the selection rule: `TROP_DATA_DIR` always wins; a home
/// variable is only ever selected when it is a non-empty absolute path, so
/// a selected default never depends on the current directory; and an
/// unusable `HOME` falls through to `USERPROFILE` rather than shadowing it.
pub proof fn lemma_data_dir_selection(
    trop_data_dir: bool,
    home: Option<HomeFacts>,
    userprofile: Option<HomeFacts>,
)
    ensures
        trop_data_dir ==> candidate_usable(trop_data_dir, home, userprofile, 0),
        candidate_usable(trop_data_dir, home, userprofile, 1) ==> (home matches Some(f)
            && f.absolute && f.nonempty),
        candidate_usable(trop_data_dir, home, userprofile, 2) ==> (userprofile matches Some(f)
            && f.absolute && f.nonempty),
        !trop_data_dir && !usable_home(home) && usable_home(userprofile) ==> candidate_usable(
            trop_data_dir,
            home,
            userprofile,
            2,
        ) && forall|j: int| 0 <= j < 2 ==> !candidate_usable(trop_data_dir, home, userprofile, j),
{
}

// ===========================================================================
// `trop init`
// ===========================================================================

/// What `trop init` observes before acting.
// One field per independently observed fact or flag.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitFacts {
    /// The data directory exists.
    pub dir_exists: bool,
    /// `trop.db` exists.
    pub db_exists: bool,
    /// `config.yaml` exists.
    pub config_exists: bool,
    /// `--overwrite`.
    pub overwrite: bool,
    /// `--with-config`.
    pub with_config: bool,
}

/// The steps of an initialization that proceeds. A database is always
/// created; `remove_db` says it replaces an existing one (with its `-wal`
/// and `-shm` files).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitSteps {
    /// Create the data directory.
    pub create_dir: bool,
    /// Remove the existing database before creating the new one.
    pub remove_db: bool,
    /// Write the default `config.yaml`.
    pub write_config: bool,
}

/// `trop init`'s decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitPlan {
    /// A database exists and `--overwrite` was not given: fail, change
    /// nothing.
    RefuseExistingDatabase,
    /// Carry out the steps.
    Proceed(InitSteps),
}

/// The state init leaves behind after carrying out `s` from facts `f`.
// A ghost type: only specifications use it. Ordinary builds erase the
// specifications but keep the declaration, which would otherwise trip the
// `dead_code` lint. One field per independent fact of the outcome.
#[allow(dead_code, clippy::struct_excessive_bools)]
pub ghost struct InitOutcome {
    /// The data directory exists.
    pub dir: bool,
    /// The database is newly created (no earlier database survives).
    pub fresh_db: bool,
    /// A configuration file exists.
    pub config: bool,
    /// An existing configuration file was left untouched.
    pub config_preserved: bool,
}

/// The outcome of carrying out `s` when the facts are `f`.
pub open spec fn outcome(f: InitFacts, s: InitSteps) -> InitOutcome {
    InitOutcome {
        dir: f.dir_exists || s.create_dir,
        fresh_db: !f.db_exists || s.remove_db,
        config: f.config_exists || s.write_config,
        config_preserved: !(f.config_exists && s.write_config),
    }
}

/// The goal: a data directory with a fresh database, and a configuration
/// file exactly when one existed or `--with-config` asked for one, never
/// replacing an existing one.
pub open spec fn achieves_goal(f: InitFacts, s: InitSteps) -> bool {
    let o = outcome(f, s);
    o.dir && o.fresh_db && o.config == (f.config_exists || f.with_config) && o.config_preserved
}

/// Safety and minimality: an existing database is removed only with
/// `--overwrite`, and no step is taken whose effect already holds or was
/// not requested.
pub open spec fn safe_and_minimal(f: InitFacts, s: InitSteps) -> bool {
    &&& s.remove_db ==> f.overwrite && f.db_exists
    &&& s.create_dir ==> !f.dir_exists
    &&& s.write_config ==> f.with_config && !f.config_exists
}

/// The plan for facts `f` (see `plan_init`).
pub open spec fn planned(f: InitFacts) -> InitPlan {
    if f.db_exists && !f.overwrite {
        InitPlan::RefuseExistingDatabase
    } else {
        InitPlan::Proceed(
            InitSteps {
                create_dir: !f.dir_exists,
                remove_db: f.db_exists,
                write_config: f.with_config && !f.config_exists,
            },
        )
    }
}

/// Plans `trop init`. The live run and `--dry-run` both use this plan, so
/// the dry run fails exactly when the live run would (on the same facts).
#[must_use]
pub fn plan_init(f: InitFacts) -> (p: InitPlan)
    ensures
        p == planned(f),
        p == InitPlan::RefuseExistingDatabase <==> (f.db_exists && !f.overwrite),
        p matches InitPlan::Proceed(s) ==> achieves_goal(f, s) && safe_and_minimal(f, s),
{
    if f.db_exists && !f.overwrite {
        InitPlan::RefuseExistingDatabase
    } else {
        InitPlan::Proceed(
            InitSteps {
                create_dir: !f.dir_exists,
                remove_db: f.db_exists,
                write_config: f.with_config && !f.config_exists,
            },
        )
    }
}

/// The goal and the safety/minimality rules determine the steps: any plan
/// meeting both is the one `plan_init` returns. So the plan is not one
/// choice among several; it is the only correct one.
pub proof fn lemma_init_steps_unique(f: InitFacts, s: InitSteps, t: InitSteps)
    requires
        achieves_goal(f, s),
        safe_and_minimal(f, s),
        achieves_goal(f, t),
        safe_and_minimal(f, t),
    ensures
        s == t,
{
}

/// Without a database there is nothing to refuse, and `--overwrite` then
/// changes nothing: the plan is the same with or without it. (So a fresh
/// directory is never reported as "recreated".)
pub proof fn lemma_overwrite_irrelevant_without_database(f: InitFacts)
    requires
        !f.db_exists,
    ensures
        planned(f) == planned(InitFacts { overwrite: !f.overwrite, ..f }),
        planned(f) matches InitPlan::Proceed(s) && !s.remove_db,
{
}

// ===========================================================================
// Assertions
// ===========================================================================

/// The verdict of an `assert-*` command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssertionOutcome {
    /// The assertion holds (exit 0).
    Holds,
    /// It fails because the thing exists (`--not` was given).
    FailsPresent,
    /// It fails because the thing does not exist.
    FailsAbsent,
}

/// The assertion "the thing exists", inverted by `--not`, holds.
pub open spec fn assertion_holds(observed: bool, negate: bool) -> bool {
    observed != negate
}

/// Decides an assertion about an observed fact. A failure names the
/// observed fact, so the failure message is always true.
#[must_use]
pub fn assertion_outcome(observed: bool, negate: bool) -> (r: AssertionOutcome)
    ensures
        r == AssertionOutcome::Holds <==> assertion_holds(observed, negate),
        r == AssertionOutcome::FailsPresent <==> observed && negate,
        r == AssertionOutcome::FailsAbsent <==> !observed && !negate,
{
    if observed != negate {
        AssertionOutcome::Holds
    } else if observed {
        AssertionOutcome::FailsPresent
    } else {
        AssertionOutcome::FailsAbsent
    }
}

/// `--not` inverts the verdict, and inverting twice restores it.
pub proof fn lemma_negation(observed: bool)
    ensures
        assertion_holds(observed, true) == !assertion_holds(observed, false),
        assertion_holds(observed, false) == observed,
{
}

/// `assert-reservation` prints the reserved port exactly when the
/// reservation exists, the assertion is not inverted, and `--quiet` is off.
#[must_use]
pub fn prints_reservation_port(exists: bool, negate: bool, quiet: bool) -> (r: bool)
    ensures
        r == (exists && !negate && !quiet),
        r ==> assertion_holds(exists, negate),
{
    exists && !negate && !quiet
}

// ===========================================================================
// Flag derivations
// ===========================================================================

/// The cleanup switches of `trop reserve`: `--disable-autoclean` turns off
/// both automatic cleanups, and each narrower flag turns off only its own.
#[must_use]
pub fn cleanup_switches(disable_autoprune: bool, disable_autoexpire: bool, disable_autoclean: bool)
    -> (r: super::reserve::CleanupSwitches)
    ensures
        r.disable_autoprune == (disable_autoprune || disable_autoclean),
        r.disable_autoexpire == (disable_autoexpire || disable_autoclean),
        disable_autoclean ==> super::reserve::cleanup_of(r) == (super::reserve::CleanupPolicy {
            prune: false,
            expire: false,
        }),
        !disable_autoclean ==> super::reserve::cleanup_of(r).prune == !disable_autoprune
            && super::reserve::cleanup_of(r).expire == !disable_autoexpire,
{
    super::reserve::CleanupSwitches {
        disable_autoprune: disable_autoprune || disable_autoclean,
        disable_autoexpire: disable_autoexpire || disable_autoclean,
    }
}

/// The metadata intent of a `reserve` field (project or task): `--clear-*`
/// clears it; otherwise an effective value (from the command line, the
/// environment or the configuration) is stored; otherwise the stored value
/// is kept. Resolved against the stored (or inferred) value, it gives
/// exactly that.
#[must_use]
pub fn metadata_intent<T: Copy>(clear: bool, value: Option<T>) -> (r: super::reserve::Intent<T>)
    ensures
        forall|fallback: Option<T>|
            #![auto]
            super::reserve::resolved(r, fallback) == if clear {
                None
            } else if value is Some {
                value
            } else {
                fallback
            },
        r == super::reserve::Intent::<T>::Clear <==> clear,
        r == super::reserve::Intent::<T>::Preserve <==> !clear && value is None,
{
    if clear {
        super::reserve::Intent::Clear
    } else {
        match value {
            Some(v) => super::reserve::Intent::Set(v),
            None => super::reserve::Intent::Preserve,
        }
    }
}

// ===========================================================================
// Occupancy layers
// ===========================================================================

/// The occupancy switches given on the command line.
// One field per command-line switch.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OccupancyFlags {
    /// `--skip-occupancy-check`.
    pub skip: bool,
    /// `--skip-tcp`.
    pub skip_tcp: bool,
    /// `--skip-udp`.
    pub skip_udp: bool,
    /// `--skip-ipv4`.
    pub skip_ipv4: bool,
    /// `--skip-ipv6`.
    pub skip_ipv6: bool,
    /// `--check-all-interfaces`.
    pub check_all_interfaces: bool,
}

/// A switch on the command line sets its leaf to `true`; an absent switch
/// leaves the leaf unset (it never writes `false`).
pub open spec fn flag_leaf(on: bool) -> Option<bool> {
    if on {
        Some(true)
    } else {
        None
    }
}

/// Some switch is given.
pub open spec fn any_flag(f: OccupancyFlags) -> bool {
    f.skip || f.skip_tcp || f.skip_udp || f.skip_ipv4 || f.skip_ipv6 || f.check_all_interfaces
}

/// The leaves the command line sets.
pub open spec fn layer_leaves(f: OccupancyFlags) -> super::occupancy::OccSettings {
    super::occupancy::OccSettings {
        skip: flag_leaf(f.skip),
        skip_tcp: flag_leaf(f.skip_tcp),
        skip_udp: flag_leaf(f.skip_udp),
        skip_ipv4: flag_leaf(f.skip_ipv4),
        skip_ipv6: flag_leaf(f.skip_ipv6),
        check_all_interfaces: flag_leaf(f.check_all_interfaces),
    }
}

/// Lays a layer over the settings below it, leaf by leaf (the configuration
/// overlay: a layer's value wins where it sets one).
pub open spec fn overlay_settings(
    lower: super::occupancy::OccSettings,
    layer: super::occupancy::OccSettings,
) -> super::occupancy::OccSettings {
    super::occupancy::OccSettings {
        skip: super::config::overlay(lower.skip, layer.skip),
        skip_tcp: super::config::overlay(lower.skip_tcp, layer.skip_tcp),
        skip_udp: super::config::overlay(lower.skip_udp, layer.skip_udp),
        skip_ipv4: super::config::overlay(lower.skip_ipv4, layer.skip_ipv4),
        skip_ipv6: super::config::overlay(lower.skip_ipv6, layer.skip_ipv6),
        check_all_interfaces: super::config::overlay(
            lower.check_all_interfaces,
            layer.check_all_interfaces,
        ),
    }
}

/// Decides `flag_leaf`.
fn flag_leaf_of(on: bool) -> (l: Option<bool>)
    ensures
        l == flag_leaf(on),
{
    if on {
        Some(true)
    } else {
        None
    }
}

/// The command line's occupancy layer: `None` when no switch is given (no
/// layer, so nothing is recorded as set on the command line), otherwise the
/// leaves of the given switches.
#[must_use]
pub fn occupancy_layer(f: OccupancyFlags) -> (r: Option<super::occupancy::OccSettings>)
    ensures
        r is None <==> !any_flag(f),
        r matches Some(s) ==> s == layer_leaves(f),
{
    if !(f.skip || f.skip_tcp || f.skip_udp || f.skip_ipv4 || f.skip_ipv6
        || f.check_all_interfaces) {
        return None;
    }
    Some(
        super::occupancy::OccSettings {
            skip: flag_leaf_of(f.skip),
            skip_tcp: flag_leaf_of(f.skip_tcp),
            skip_udp: flag_leaf_of(f.skip_udp),
            skip_ipv4: flag_leaf_of(f.skip_ipv4),
            skip_ipv6: flag_leaf_of(f.skip_ipv6),
            check_all_interfaces: flag_leaf_of(f.check_all_interfaces),
        },
    )
}

/// The command-line layer only ever disables probes: over any lower
/// settings, a probe it leaves requested was already requested below,
/// except the wildcard probes `--check-all-interfaces` adds. Every given
/// skip switch disables its probes whatever the lower layers say.
pub proof fn lemma_layer_only_disables(
    lower: super::occupancy::OccSettings,
    f: OccupancyFlags,
    probe: super::occupancy::Probe,
)
    ensures
        super::occupancy::settings_request_probe(overlay_settings(lower, layer_leaves(f)), probe)
            ==> super::occupancy::settings_request_probe(lower, probe) || (probe.scope
            == super::occupancy::Scope::Wildcard && f.check_all_interfaces),
        f.skip ==> !super::occupancy::settings_request_probe(
            overlay_settings(lower, layer_leaves(f)),
            probe,
        ),
        f.skip_tcp && probe.protocol == super::occupancy::Protocol::Tcp
            ==> !super::occupancy::settings_request_probe(
            overlay_settings(lower, layer_leaves(f)),
            probe,
        ),
        f.skip_udp && probe.protocol == super::occupancy::Protocol::Udp
            ==> !super::occupancy::settings_request_probe(
            overlay_settings(lower, layer_leaves(f)),
            probe,
        ),
        f.skip_ipv4 && probe.family == super::occupancy::Family::Ipv4
            ==> !super::occupancy::settings_request_probe(
            overlay_settings(lower, layer_leaves(f)),
            probe,
        ),
        f.skip_ipv6 && probe.family == super::occupancy::Family::Ipv6
            ==> !super::occupancy::settings_request_probe(
            overlay_settings(lower, layer_leaves(f)),
            probe,
        ),
{
}

/// The settings `trop scan` probes with: the configured settings without the
/// global `skip` switch. Scanning is the command's purpose (it has no
/// `--skip-occupancy-check`), so a configured `skip: true` meant for
/// allocations does not turn the scan into a no-op; the per-protocol and
/// per-family switches still apply.
pub open spec fn scan_settings(s: super::occupancy::OccSettings) -> super::occupancy::OccSettings {
    super::occupancy::OccSettings { skip: None, ..s }
}

/// What `trop scan` does under the effective settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanProbes {
    /// Every probe is disabled: the ports cannot be checked, and the scan
    /// must say so rather than report that nothing is occupied.
    NotChecked,
    /// Probe with these flags (at least one probe is enabled).
    Probe(super::occupancy::OccFlags),
}

/// Decides `trop scan`'s probes from the effective settings.
#[must_use]
pub fn scan_probes(s: super::occupancy::OccSettings) -> (r: ScanProbes)
    ensures
        r == ScanProbes::NotChecked <==> super::occupancy::no_probe_enabled(
            super::occupancy::spec_effective_flags(scan_settings(s)),
        ),
        r matches ScanProbes::Probe(flags) ==> flags == super::occupancy::spec_effective_flags(
            scan_settings(s),
        ) && forall|probe: super::occupancy::Probe|
            #[trigger] super::occupancy::probe_enabled(flags, probe)
                <==> super::occupancy::settings_request_probe(scan_settings(s), probe),
{
    let flags = super::occupancy::effective_occupancy_flags(
        super::occupancy::OccSettings { skip: None, ..s },
    );
    if super::occupancy::occupancy_disabled(flags) {
        ScanProbes::NotChecked
    } else {
        ScanProbes::Probe(flags)
    }
}

/// `scan` ignores the global `skip` switch and nothing else: its probes do
/// not depend on `skip`; without a configured `skip: true` it probes exactly
/// what an allocation would; and with no protocol or family skipped it
/// always probes (at least the localhost TCP/UDP x IPv4/IPv6 probes).
pub proof fn lemma_scan_ignores_global_skip(
    s: super::occupancy::OccSettings,
    skip: Option<bool>,
    probe: super::occupancy::Probe,
)
    ensures
        scan_settings(s) == scan_settings(super::occupancy::OccSettings { skip, ..s }),
        s.skip != Some(true) ==> (super::occupancy::settings_request_probe(scan_settings(s), probe)
            <==> super::occupancy::settings_request_probe(s, probe)),
        s.skip_tcp != Some(true) && s.skip_udp != Some(true) && s.skip_ipv4 != Some(true)
            && s.skip_ipv6 != Some(true) && probe.scope == super::occupancy::Scope::Localhost
            ==> super::occupancy::settings_request_probe(scan_settings(s), probe),
{
}

// ===========================================================================
// List filters
// ===========================================================================

/// The `list` filter `filter` selects the stored identifier `stored`: the
/// filter is compared in canonical (trimmed) form, the form identifiers are
/// stored in, and a blank filter selects nothing.
pub open spec fn filter_selects(filter: Seq<char>, stored: Option<Seq<char>>) -> bool {
    let canonical = super::identifier::trimmed(filter);
    canonical.len() > 0 && stored == Some(canonical)
}

/// Decides `filter_selects` (for `--filter-project` and `--filter-tag`).
#[must_use]
pub fn identifier_filter_matches(filter: &str, stored: Option<&str>) -> (r: bool)
    ensures
        r == filter_selects(
            filter@,
            match stored {
                Some(s) => Some(s@),
                None => None,
            },
        ),
{
    let canonical = super::identifier::trim(filter);
    if canonical.is_empty() {
        return false;
    }
    match stored {
        Some(s) => {
            let same = super::text::bytes_equal(s.as_bytes(), canonical.as_bytes());
            proof {
                super::config::lemma_utf8_injective(s@, canonical@);
            }
            same
        },
        None => false,
    }
}

/// The filter behaves like the stored identifier rule: whitespace padding
/// around a filter does not matter, a blank filter selects nothing, and a
/// stored canonical identifier is selected by its own text (and by any
/// padded form of it).
pub proof fn lemma_filter_canonical(filter: Seq<char>, pad: Seq<char>, stored: Seq<char>)
    requires
        super::identifier::all_ws(pad),
    ensures
        filter_selects(pad + filter + pad, Some(stored)) == filter_selects(filter, Some(stored)),
        super::identifier::all_ws(filter) ==> !filter_selects(filter, Some(stored)),
        super::identifier::is_canonical(stored) ==> filter_selects(stored, Some(stored))
            && filter_selects(pad + stored + pad, Some(stored)),
{
    super::identifier::lemma_trimmed_padding(pad, filter, pad);
    super::identifier::lemma_trimmed_empty_iff_all_ws(filter);
    super::identifier::lemma_trimmed_padding(pad, stored, pad);
}

// ===========================================================================
// Port arguments
// ===========================================================================

/// Why a port argument was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortArgError {
    /// Not an unsigned decimal number in `u32` range.
    NotANumber,
    /// The number 0.
    Zero,
    /// A number above 65535.
    TooLarge(u32),
}

/// `s` is the text of a usable port: `u16::from_str` syntax (optional `+`,
/// ASCII digits) with a value from 1 to 65535.
pub open spec fn is_port_text(s: Seq<u8>, p: int) -> bool {
    1 <= p && super::exclusions::is_unsigned_text(s, p, 65535)
}

/// Parses a port argument (`--port`, `--min`, `--max`, a `PORT` operand).
///
/// The text is read as a `u32` first, so the error says whether it is not a
/// number, zero, or too large; this precedence matches `trop reserve`.
pub fn parse_port_arg(s: &[u8]) -> (r: Result<u16, PortArgError>)
    ensures
        r matches Ok(p) ==> is_port_text(s@, p as int),
        r is Ok <==> exists|p: int| is_port_text(s@, p),
        r == Err::<u16, PortArgError>(PortArgError::NotANumber) <==> forall|v: int|
            !super::exclusions::is_unsigned_text(s@, v, u32::MAX as int),
        r == Err::<u16, PortArgError>(PortArgError::Zero) <==> super::exclusions::is_unsigned_text(
            s@,
            0,
            u32::MAX as int,
        ),
        r matches Err(PortArgError::TooLarge(v)) ==> v > 65535
            && super::exclusions::is_unsigned_text(s@, v as int, u32::MAX as int),
{
    match super::config::parse_u32_decimal(s) {
        None => {
            proof {
                if exists|p: int| is_port_text(s@, p) {
                    let p = choose|p: int| is_port_text(s@, p);
                    assert(super::exclusions::is_unsigned_text(s@, p, u32::MAX as int));
                }
            }
            Err(PortArgError::NotANumber)
        },
        Some(v) => {
            proof {
                assert forall|w: int|
                    #[trigger] super::exclusions::is_unsigned_text(s@, w, u32::MAX as int)
                        implies w == v as int by {}
                assert forall|w: int|
                    #[trigger] super::exclusions::is_unsigned_text(s@, w, 65535) implies w
                    == v as int by {}
            }
            if v == 0 {
                Err(PortArgError::Zero)
            } else {
                match u16::try_from(v) {
                    Ok(p) => {
                        proof {
                            assert(is_port_text(s@, p as int));
                        }
                        Ok(p)
                    },
                    Err(_) => Err(PortArgError::TooLarge(v)),
                }
            }
        },
    }
}

} // verus!
