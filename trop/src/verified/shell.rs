//! Verified kernel: shell names, shell detection, and export lines.
//!
//! - `parse_shell_name` is the `--shell` alias table (`bash`/`sh`, `zsh`,
//!   `fish`, `powershell`/`pwsh`/`ps`), compared without ASCII case and with
//!   no trimming or prefix matching; the alias sets are disjoint.
//! - `classify_shell_path` classifies a `SHELL` value by its final path
//!   component only (`basename`, after the last `/` or `\`), with a trailing
//!   `.exe` removed: directory names such as `/home/selfish/` can never
//!   change the result (`lemma_basename_ignores_directory`). The result
//!   table is `shell_program` (`lemma_program_table_posix`,
//!   `lemma_program_table_others`, `lemma_selfish_bash_is_bash`).
//! - `detect_shell` keeps the version-variable precedence (`ZSH_VERSION`,
//!   then `FISH_VERSION`, then `PSModulePath`) and otherwise classifies
//!   `SHELL`; `None` means the shell cannot be determined, which production
//!   reports as an error asking for `--shell`.
//! - `push_export_line` / `export_line` render `export N=V`, `set -x N V`,
//!   or `$env:N="V"`. For a portable name and an inert value
//!   (`is_inert_value`: a non-empty word of `[A-Za-z0-9_./:@+,-]` that does
//!   not start with `-`), the line is one simple command under a stated
//!   model of each shell's lexer (`simple_words`, `ps_assignment`), it
//!   contains no shell metacharacter outside the fixed template, and it
//!   parses back to exactly its name and value (`lemma_export_round_trip`),
//!   so distinct assignments give distinct lines.
//!
//! The model lexers are a conservative abstraction of the real grammars: a
//! line made only of the listed word bytes and single spaces is assumed to
//! be the space-separated words with no expansion. This assumption is
//! documented in `docs/verification.md`, not proved.
//!
//! Production glue: `crate::output::ShellType`.

use vstd::prelude::*;

use super::env_name;
use super::render;
use super::text;

verus! {

broadcast use vstd::seq_lib::group_seq_properties;

/// The shells whose export syntax trop can render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellKind {
    /// POSIX `export` syntax (bash, sh, dash, ksh, ...).
    Bash,
    /// Z shell (same syntax as bash).
    Zsh,
    /// Friendly interactive shell.
    Fish,
    /// `PowerShell`.
    PowerShell,
}

// ---------------------------------------------------------------------------
// Literals
// ---------------------------------------------------------------------------

/// `bash`
pub open spec fn lit_bash() -> Seq<u8> {
    seq![b'b', b'a', b's', b'h']
}

/// `sh`
pub open spec fn lit_sh() -> Seq<u8> {
    seq![b's', b'h']
}

/// `zsh`
pub open spec fn lit_zsh() -> Seq<u8> {
    seq![b'z', b's', b'h']
}

/// `fish`
pub open spec fn lit_fish() -> Seq<u8> {
    seq![b'f', b'i', b's', b'h']
}

/// `powershell`
pub open spec fn lit_powershell() -> Seq<u8> {
    seq![b'p', b'o', b'w', b'e', b'r', b's', b'h', b'e', b'l', b'l']
}

/// `pwsh`
pub open spec fn lit_pwsh() -> Seq<u8> {
    seq![b'p', b'w', b's', b'h']
}

/// `ps`
pub open spec fn lit_ps() -> Seq<u8> {
    seq![b'p', b's']
}

/// `dash`
pub open spec fn lit_dash() -> Seq<u8> {
    seq![b'd', b'a', b's', b'h']
}

/// `ksh`
pub open spec fn lit_ksh() -> Seq<u8> {
    seq![b'k', b's', b'h']
}

/// `mksh`
pub open spec fn lit_mksh() -> Seq<u8> {
    seq![b'm', b'k', b's', b'h']
}

/// `ash`
pub open spec fn lit_ash() -> Seq<u8> {
    seq![b'a', b's', b'h']
}

/// `yash`
pub open spec fn lit_yash() -> Seq<u8> {
    seq![b'y', b'a', b's', b'h']
}

/// `.exe`
pub open spec fn lit_exe() -> Seq<u8> {
    seq![b'.', b'e', b'x', b'e']
}

/// Equality ignoring ASCII case.
pub open spec fn ci(a: Seq<u8>, b: Seq<u8>) -> bool {
    text::eq_ignore_ascii_case(a, b)
}

// ---------------------------------------------------------------------------
// `--shell` names
// ---------------------------------------------------------------------------

/// The `--shell` aliases of each shell.
pub open spec fn aliases(k: ShellKind) -> Seq<Seq<u8>> {
    match k {
        ShellKind::Bash => seq![lit_bash(), lit_sh()],
        ShellKind::Zsh => seq![lit_zsh()],
        ShellKind::Fish => seq![lit_fish()],
        ShellKind::PowerShell => seq![lit_powershell(), lit_pwsh(), lit_ps()],
    }
}

/// The shell named by a `--shell` value, compared without ASCII case.
pub open spec fn shell_alias(s: Seq<u8>) -> Option<ShellKind> {
    if ci(s, lit_bash()) || ci(s, lit_sh()) {
        Some(ShellKind::Bash)
    } else if ci(s, lit_zsh()) {
        Some(ShellKind::Zsh)
    } else if ci(s, lit_fish()) {
        Some(ShellKind::Fish)
    } else if ci(s, lit_powershell()) || ci(s, lit_pwsh()) || ci(s, lit_ps()) {
        Some(ShellKind::PowerShell)
    } else {
        None
    }
}

/// `b` is an ASCII letter.
pub open spec fn is_letter(b: u8) -> bool {
    text::is_ascii_lower_byte(b) || text::is_ascii_upper_byte(b)
}

/// Every alias is 2 to 10 lowercase ASCII letters.
proof fn lemma_aliases_are_letters(k: ShellKind)
    ensures
        forall|i: int, j: int|
            0 <= i < aliases(k).len() && 0 <= j < aliases(k)[i].len() ==> text::is_ascii_lower_byte(
                #[trigger] aliases(k)[i][j],
            ),
        forall|i: int| 0 <= i < aliases(k).len() ==> 2 <= #[trigger] aliases(k)[i].len() <= 10,
{
}

/// The index, within `aliases(k)`, of the alias that `shell_alias` matched.
pub open spec fn matched_alias(s: Seq<u8>) -> int {
    if ci(s, lit_bash()) {
        0
    } else if ci(s, lit_sh()) {
        1
    } else if ci(s, lit_zsh()) || ci(s, lit_fish()) || ci(s, lit_powershell()) {
        0
    } else if ci(s, lit_pwsh()) {
        1
    } else {
        2
    }
}

/// An accepted string equals its matched alias ignoring case.
proof fn lemma_matched_alias(s: Seq<u8>)
    requires
        shell_alias(s) is Some,
    ensures
        0 <= matched_alias(s) < aliases(shell_alias(s)->Some_0).len(),
        ci(s, aliases(shell_alias(s)->Some_0)[matched_alias(s)]),
{
}

/// An accepted string is 2 to 10 ASCII letters.
proof fn lemma_accepted_are_letters(s: Seq<u8>)
    requires
        shell_alias(s) is Some,
    ensures
        2 <= s.len() <= 10,
        forall|j: int| 0 <= j < s.len() ==> is_letter(#[trigger] s[j]),
{
    let k = shell_alias(s)->Some_0;
    lemma_matched_alias(s);
    lemma_aliases_are_letters(k);
    let a = aliases(k)[matched_alias(s)];
    assert forall|j: int| 0 <= j < s.len() implies is_letter(#[trigger] s[j]) by {
        assert(text::ascii_upper(s[j]) == text::ascii_upper(a[j]));
        assert(text::is_ascii_lower_byte(a[j]));
    }
}

/// The alias table is exact: `s` parses to `k` exactly when it equals one of
/// `k`'s aliases ignoring ASCII case. The alias sets are disjoint even
/// ignoring case, and every accepted string is 2 to 10 ASCII letters, so
/// empty, padded or prefixed input is rejected.
pub proof fn lemma_shell_alias_exact(s: Seq<u8>, k: ShellKind)
    ensures
        shell_alias(s) == Some(k) <==> exists|i: int|
            0 <= i < aliases(k).len() && #[trigger] ci(s, aliases(k)[i]),
        shell_alias(s) is Some ==> 2 <= s.len() <= 10 && forall|j: int|
            0 <= j < s.len() ==> is_letter(#[trigger] s[j]),
{
    if shell_alias(s) is Some {
        lemma_matched_alias(s);
        lemma_accepted_are_letters(s);
    }
    if exists|i: int| 0 <= i < aliases(k).len() && #[trigger] ci(s, aliases(k)[i]) {
        let i = choose|i: int| 0 <= i < aliases(k).len() && #[trigger] ci(s, aliases(k)[i]);
        lemma_parses_to_owner(s, k, i);
    }
}

/// A string equal (ignoring case) to an alias of `k` parses to `k`.
proof fn lemma_parses_to_owner(s: Seq<u8>, k: ShellKind, i: int)
    requires
        0 <= i < aliases(k).len(),
        ci(s, aliases(k)[i]),
    ensures
        shell_alias(s) == Some(k),
{
    lemma_aliases_disjoint();
    let a = aliases(k)[i];
    assert forall|k2: ShellKind, i2: int|
        0 <= i2 < aliases(k2).len() && ci(s, #[trigger] aliases(k2)[i2]) implies k2 == k by {
        text::lemma_eq_ignore_ascii_case_equivalence(s, a, aliases(k2)[i2]);
    }
    if shell_alias(s) is Some {
        lemma_matched_alias(s);
    } else {
        // `s` matches no alias, but it matches `a`.
        assert(ci(s, aliases(k)[i]));
        if k == ShellKind::Bash {
            assert(i == 0 || i == 1);
        }
        if k == ShellKind::PowerShell {
            assert(i == 0 || i == 1 || i == 2);
        }
    }
}

/// No alias of one shell equals an alias of another, ignoring ASCII case.
proof fn lemma_aliases_disjoint()
    ensures
        forall|k1: ShellKind, k2: ShellKind, i1: int, i2: int|
            0 <= i1 < aliases(k1).len() && 0 <= i2 < aliases(k2).len() && ci(
                #[trigger] aliases(k1)[i1],
                #[trigger] aliases(k2)[i2],
            ) ==> k1 == k2,
{
    assert forall|k1: ShellKind, k2: ShellKind, i1: int, i2: int|
        0 <= i1 < aliases(k1).len() && 0 <= i2 < aliases(k2).len() && ci(
            #[trigger] aliases(k1)[i1],
            #[trigger] aliases(k2)[i2],
        ) implies k1 == k2 by {
        let a = aliases(k1)[i1];
        let b = aliases(k2)[i2];
        // Aliases of equal length differ (ignoring case) in their first or
        // second byte.
        assert(text::ascii_upper(a[0]) == text::ascii_upper(b[0]));
        assert(text::ascii_upper(a[1]) == text::ascii_upper(b[1]));
    }
}

/// Parsing ignores ASCII case.
pub proof fn lemma_shell_alias_case_insensitive(s: Seq<u8>, t: Seq<u8>)
    requires
        ci(s, t),
    ensures
        shell_alias(s) == shell_alias(t),
{
    text::lemma_eq_ignore_ascii_case_equivalence(s, t, t);
    assert forall|a: Seq<u8>| #[trigger] ci(s, a) <==> ci(t, a) by {
        text::lemma_eq_ignore_ascii_case_equivalence(s, t, a);
        text::lemma_eq_ignore_ascii_case_equivalence(t, s, a);
    }
}

// ---------------------------------------------------------------------------
// `SHELL` classification
// ---------------------------------------------------------------------------

/// `/` or `\`.
pub open spec fn is_path_separator(b: u8) -> bool {
    b == b'/' || b == b'\\'
}

/// Where the final path component of `p` starts: just after the last
/// separator, or 0.
pub open spec fn basename_start(p: Seq<u8>) -> int
    decreases p.len(),
{
    if p.len() == 0 {
        0
    } else if is_path_separator(p.last()) {
        p.len() as int
    } else {
        basename_start(p.drop_last())
    }
}

/// The final path component of `p`: the bytes after the last `/` or `\`.
pub open spec fn basename(p: Seq<u8>) -> Seq<u8> {
    p.subrange(basename_start(p), p.len() as int)
}

/// The basename starts after the last separator: it is a separator-free
/// suffix of `p`, preceded by a separator unless it is all of `p`.
pub proof fn lemma_basename_shape(p: Seq<u8>)
    ensures
        0 <= basename_start(p) <= p.len(),
        basename_start(p) > 0 ==> is_path_separator(p[basename_start(p) - 1]),
        forall|i: int| basename_start(p) <= i < p.len() ==> !is_path_separator(#[trigger] p[i]),
    decreases p.len(),
{
    if p.len() > 0 && !is_path_separator(p.last()) {
        lemma_basename_shape(p.drop_last());
        assert forall|i: int| basename_start(p) <= i < p.len() implies !is_path_separator(
            #[trigger] p[i],
        ) by {
            if i < p.len() - 1 {
                assert(p[i] == p.drop_last()[i]);
            }
        }
        if basename_start(p) > 0 {
            assert(p[basename_start(p) - 1] == p.drop_last()[basename_start(p) - 1]);
        }
    }
}

/// A component without separators is its own basename.
pub proof fn lemma_basename_of_component(c: Seq<u8>)
    requires
        forall|i: int| 0 <= i < c.len() ==> !is_path_separator(#[trigger] c[i]),
    ensures
        basename_start(c) == 0,
        basename(c) == c,
    decreases c.len(),
{
    if c.len() > 0 {
        assert(!is_path_separator(c[c.len() - 1]));
        assert forall|i: int| 0 <= i < c.drop_last().len() implies !is_path_separator(
            #[trigger] c.drop_last()[i],
        ) by {
            assert(c.drop_last()[i] == c[i]);
        }
        lemma_basename_of_component(c.drop_last());
    }
    assert(c.subrange(0, c.len() as int) =~= c);
}

/// Directory components never matter: after any prefix that ends in a
/// separator, the basename is the basename of the rest. So
/// `/home/selfish/bin/bash` and `/opt/powershell-addons/bin/bash` have the
/// basename `bash`.
pub proof fn lemma_basename_ignores_directory(dir: Seq<u8>, rest: Seq<u8>)
    requires
        dir.len() > 0,
        is_path_separator(dir.last()),
    ensures
        basename(dir + rest) == basename(rest),
    decreases rest.len(),
{
    let p = dir + rest;
    if rest.len() == 0 {
        assert(p =~= dir);
        assert(basename(p) =~= Seq::<u8>::empty());
    } else if is_path_separator(rest.last()) {
        assert(p.last() == rest.last());
        assert(basename(p) =~= Seq::<u8>::empty());
    } else {
        lemma_basename_ignores_directory(dir, rest.drop_last());
        assert(p.drop_last() =~= dir + rest.drop_last());
        assert(p.last() == rest.last());
        lemma_basename_shape(p.drop_last());
        lemma_basename_shape(rest.drop_last());
        let s1 = basename_start(p);
        let s2 = basename_start(rest);
        assert(basename(p) =~= basename(p.drop_last()).push(p.last()));
        assert(basename(rest) =~= basename(rest.drop_last()).push(rest.last()));
    }
}

/// A trailing `.exe` (any ASCII case), as on Windows.
pub open spec fn has_exe_suffix(name: Seq<u8>) -> bool {
    name.len() >= 4 && ci(name.subrange(name.len() - 4, name.len() as int), lit_exe())
}

/// The program name without a trailing `.exe`.
pub open spec fn program_stem(name: Seq<u8>) -> Seq<u8> {
    if has_exe_suffix(name) {
        name.subrange(0, name.len() - 4)
    } else {
        name
    }
}

/// The shell a program name denotes, ignoring ASCII case: `zsh`; `fish`;
/// `pwsh` or `powershell`; or a POSIX shell (`bash`, `sh`, `dash`, `ksh`,
/// `mksh`, `ash`, `yash`), which takes `export` syntax. Anything else is
/// unknown.
pub open spec fn shell_program(stem: Seq<u8>) -> Option<ShellKind> {
    if ci(stem, lit_zsh()) {
        Some(ShellKind::Zsh)
    } else if ci(stem, lit_fish()) {
        Some(ShellKind::Fish)
    } else if ci(stem, lit_pwsh()) || ci(stem, lit_powershell()) {
        Some(ShellKind::PowerShell)
    } else if ci(stem, lit_bash()) || ci(stem, lit_sh()) || ci(stem, lit_dash()) || ci(
        stem,
        lit_ksh(),
    ) || ci(stem, lit_mksh()) || ci(stem, lit_ash()) || ci(stem, lit_yash()) {
        Some(ShellKind::Bash)
    } else {
        None
    }
}

/// The classification of a `SHELL` value: a function of its basename only.
pub open spec fn classify_shell_path_spec(p: Seq<u8>) -> Option<ShellKind> {
    shell_program(program_stem(basename(p)))
}

/// The classification ignores directories.
pub proof fn lemma_classification_ignores_directory(dir: Seq<u8>, rest: Seq<u8>)
    requires
        dir.len() > 0,
        is_path_separator(dir.last()),
    ensures
        classify_shell_path_spec(dir + rest) == classify_shell_path_spec(rest),
{
    lemma_basename_ignores_directory(dir, rest);
}

/// A literal equals itself ignoring case.
proof fn lemma_ci_refl(s: Seq<u8>)
    ensures
        ci(s, s),
{
    text::lemma_eq_ignore_ascii_case_equivalence(s, s, s);
}

/// `name + ".exe"` has the stem `name`.
proof fn lemma_exe_stem(name: Seq<u8>)
    ensures
        program_stem(name + lit_exe()) == name,
{
    let full = name + lit_exe();
    assert(full.subrange(full.len() - 4, full.len() as int) =~= lit_exe());
    lemma_ci_refl(lit_exe());
    assert(full.subrange(0, full.len() - 4) =~= name);
}

/// The result table for POSIX shell names: each classifies as `Bash`, with
/// or without `.exe`.
pub proof fn lemma_program_table_posix()
    ensures
        forall|name: Seq<u8>|
            #![trigger program_stem(name)]
            name == lit_bash() || name == lit_sh() || name == lit_dash() || name == lit_ksh()
                || name == lit_mksh() || name == lit_ash() || name == lit_yash() ==> shell_program(
                program_stem(name),
            ) == Some(ShellKind::Bash) && shell_program(program_stem(name + lit_exe()))
                == Some(ShellKind::Bash),
{
    assert forall|name: Seq<u8>|
        #![trigger program_stem(name)]
        name == lit_bash() || name == lit_sh() || name == lit_dash() || name == lit_ksh() || name
            == lit_mksh() || name == lit_ash() || name == lit_yash() implies shell_program(
        program_stem(name),
    ) == Some(ShellKind::Bash) && shell_program(program_stem(name + lit_exe())) == Some(
        ShellKind::Bash,
    ) by {
        lemma_exe_stem(name);
        lemma_ci_refl(name);
        if name.len() >= 4 {
            assert(text::ascii_upper(name[name.len() - 4]) != text::ascii_upper(lit_exe()[0]));
        }
        assert(!ci(name, lit_zsh()) || name.len() != 3 || text::ascii_upper(name[0]) != b'Z');
        assert(!ci(name, lit_zsh()));
        assert(!ci(name, lit_fish()));
        assert(!ci(name, lit_pwsh()));
        assert(!ci(name, lit_powershell()));
    }
}

/// `zsh` classifies as `Zsh`, with or without `.exe`.
proof fn lemma_table_zsh()
    ensures
        shell_program(program_stem(lit_zsh())) == Some(ShellKind::Zsh),
        shell_program(program_stem(lit_zsh() + lit_exe())) == Some(ShellKind::Zsh),
{
    lemma_exe_stem(lit_zsh());
    lemma_ci_refl(lit_zsh());
}

/// `fish` classifies as `Fish`, with or without `.exe`.
proof fn lemma_table_fish()
    ensures
        shell_program(program_stem(lit_fish())) == Some(ShellKind::Fish),
        shell_program(program_stem(lit_fish() + lit_exe())) == Some(ShellKind::Fish),
{
    let f = lit_fish();
    lemma_exe_stem(f);
    lemma_ci_refl(f);
    assert(f.subrange(0, 4) =~= f);
    assert(text::ascii_upper(f[0]) != text::ascii_upper(lit_exe()[0]));
    assert(!has_exe_suffix(f));
    assert(!ci(f, lit_zsh()));
}

/// `pwsh` classifies as `PowerShell`, with or without `.exe`.
proof fn lemma_table_pwsh()
    ensures
        shell_program(program_stem(lit_pwsh())) == Some(ShellKind::PowerShell),
        shell_program(program_stem(lit_pwsh() + lit_exe())) == Some(ShellKind::PowerShell),
{
    let p = lit_pwsh();
    lemma_exe_stem(p);
    lemma_ci_refl(p);
    assert(p.subrange(0, 4) =~= p);
    assert(text::ascii_upper(p[0]) != text::ascii_upper(lit_exe()[0]));
    assert(!has_exe_suffix(p));
    assert(!ci(p, lit_zsh()));
    assert(text::ascii_upper(p[0]) != text::ascii_upper(lit_fish()[0]));
    assert(!ci(p, lit_fish()));
}

/// `powershell` classifies as `PowerShell`, with or without `.exe`.
proof fn lemma_table_powershell()
    ensures
        shell_program(program_stem(lit_powershell())) == Some(ShellKind::PowerShell),
        shell_program(program_stem(lit_powershell() + lit_exe())) == Some(ShellKind::PowerShell),
{
    let q = lit_powershell();
    lemma_exe_stem(q);
    lemma_ci_refl(q);
    assert(q.subrange(q.len() - 4, q.len() as int)[0] == q[6]);
    assert(text::ascii_upper(q[6]) != text::ascii_upper(lit_exe()[0]));
    assert(!has_exe_suffix(q));
    assert(!ci(q, lit_zsh()));
    assert(!ci(q, lit_fish()));
    assert(!ci(q, lit_pwsh()));
}

/// The result table for the other shells, with or without `.exe`.
pub proof fn lemma_program_table_others()
    ensures
        shell_program(program_stem(lit_zsh())) == Some(ShellKind::Zsh),
        shell_program(program_stem(lit_zsh() + lit_exe())) == Some(ShellKind::Zsh),
        shell_program(program_stem(lit_fish())) == Some(ShellKind::Fish),
        shell_program(program_stem(lit_fish() + lit_exe())) == Some(ShellKind::Fish),
        shell_program(program_stem(lit_pwsh())) == Some(ShellKind::PowerShell),
        shell_program(program_stem(lit_pwsh() + lit_exe())) == Some(ShellKind::PowerShell),
        shell_program(program_stem(lit_powershell())) == Some(ShellKind::PowerShell),
        shell_program(program_stem(lit_powershell() + lit_exe())) == Some(ShellKind::PowerShell),
{
    lemma_table_zsh();
    lemma_table_fish();
    lemma_table_pwsh();
    lemma_table_powershell();
}

/// The motivating example: `SHELL=/home/selfish/bin/bash` is bash (it used
/// to be classified as fish because the path contains "fish").
pub proof fn lemma_selfish_bash_is_bash()
    ensures
        classify_shell_path_spec(
            seq![
                b'/', b'h', b'o', b'm', b'e', b'/', b's', b'e', b'l', b'f', b'i', b's', b'h', b'/',
                b'b', b'i', b'n', b'/', b'b', b'a', b's', b'h',
            ],
        ) == Some(ShellKind::Bash),
{
    let dir = seq![
        b'/', b'h', b'o', b'm', b'e', b'/', b's', b'e', b'l', b'f', b'i', b's', b'h', b'/',
        b'b', b'i', b'n', b'/',
    ];
    let path = seq![
        b'/', b'h', b'o', b'm', b'e', b'/', b's', b'e', b'l', b'f', b'i', b's', b'h', b'/',
        b'b', b'i', b'n', b'/', b'b', b'a', b's', b'h',
    ];
    assert(path =~= dir + lit_bash());
    lemma_classification_ignores_directory(dir, lit_bash());
    assert forall|i: int| 0 <= i < lit_bash().len() implies !is_path_separator(
        #[trigger] lit_bash()[i],
    ) by {}
    lemma_basename_of_component(lit_bash());
    lemma_program_table_posix();
    assert(program_stem(lit_bash()) == program_stem(basename(lit_bash())));
}

/// Which shell-specific variables are set in the environment.
#[derive(Clone, Copy, Debug)]
pub struct ShellSignals {
    /// `ZSH_VERSION` is set.
    pub zsh_version: bool,
    /// `FISH_VERSION` is set.
    pub fish_version: bool,
    /// `PSModulePath` is set.
    pub ps_module_path: bool,
}

/// Detection: `ZSH_VERSION`, then `FISH_VERSION`, then `PSModulePath`, then
/// the basename of `SHELL`. `None` means undetermined.
pub open spec fn detected_shell(signals: ShellSignals, shell_var: Option<Seq<u8>>) -> Option<
    ShellKind,
> {
    if signals.zsh_version {
        Some(ShellKind::Zsh)
    } else if signals.fish_version {
        Some(ShellKind::Fish)
    } else if signals.ps_module_path {
        Some(ShellKind::PowerShell)
    } else {
        match shell_var {
            Some(p) => classify_shell_path_spec(p),
            None => None,
        }
    }
}

/// Detection fails exactly when no version variable is set and `SHELL` is
/// unset or names no known shell.
pub proof fn lemma_detection_undetermined(signals: ShellSignals, shell_var: Option<Seq<u8>>)
    ensures
        detected_shell(signals, shell_var) is None <==> !signals.zsh_version
            && !signals.fish_version && !signals.ps_module_path && match shell_var {
            Some(p) => classify_shell_path_spec(p) is None,
            None => true,
        },
{
}

// ---------------------------------------------------------------------------
// Export lines and the model lexers
// ---------------------------------------------------------------------------

/// `export ` (with the trailing space).
pub open spec fn lit_export_sp() -> Seq<u8> {
    seq![b'e', b'x', b'p', b'o', b'r', b't', b' ']
}

/// `set -x ` (with the trailing space).
pub open spec fn lit_set_x_sp() -> Seq<u8> {
    seq![b's', b'e', b't', b' ', b'-', b'x', b' ']
}

/// `$env:`
pub open spec fn lit_env_prefix() -> Seq<u8> {
    seq![b'$', b'e', b'n', b'v', b':']
}

/// The word `export`.
pub open spec fn word_export() -> Seq<u8> {
    seq![b'e', b'x', b'p', b'o', b'r', b't']
}

/// The word `set`.
pub open spec fn word_set() -> Seq<u8> {
    seq![b's', b'e', b't']
}

/// The word `-x`.
pub open spec fn word_dash_x() -> Seq<u8> {
    seq![b'-', b'x']
}

/// The export statement for `name` and `value` in each shell's syntax.
pub open spec fn export_line_spec(sh: ShellKind, name: Seq<u8>, value: Seq<u8>) -> Seq<u8> {
    match sh {
        ShellKind::Bash | ShellKind::Zsh => lit_export_sp() + name + seq![b'='] + value,
        ShellKind::Fish => lit_set_x_sp() + name + seq![b' '] + value,
        ShellKind::PowerShell => lit_env_prefix() + name + seq![b'=', b'"'] + value + seq![b'"'],
    }
}

/// A byte that may appear in an exported value: an ASCII letter or digit, or
/// one of `_ . / : @ + , -`.
pub open spec fn is_value_byte(b: u8) -> bool {
    ||| is_letter(b)
    ||| (b'0' <= b <= b'9')
    ||| b == b'_'
    ||| b == b'.'
    ||| b == b'/'
    ||| b == b':'
    ||| b == b'@'
    ||| b == b'+'
    ||| b == b','
    ||| b == b'-'
}

/// A value that every supported shell reads literally: a non-empty word of
/// value bytes that does not start with `-` (so fish's `set` cannot read it
/// as an option). Every port's decimal text is inert.
pub open spec fn is_inert_value(v: Seq<u8>) -> bool {
    &&& v.len() > 0
    &&& v[0] != b'-'
    &&& forall|i: int| 0 <= i < v.len() ==> #[trigger] is_value_byte(v[i])
}

/// A byte the model lexers read literally inside a word: a value byte or `=`.
pub open spec fn is_word_byte(b: u8) -> bool {
    is_value_byte(b) || b == b'='
}

/// Shell metacharacters: bytes that a POSIX shell, fish, or `PowerShell` may
/// treat as a separator, operator, quote, escape, expansion or comment, plus
/// every control and non-ASCII byte.
pub open spec fn is_shell_metachar(b: u8) -> bool {
    ||| b < 0x20
    ||| b == 0x7F
    ||| b >= 0x80
    ||| b == b' '
    ||| b == b'"'
    ||| b == b'\''
    ||| b == b'`'
    ||| b == b'$'
    ||| b == b';'
    ||| b == b'&'
    ||| b == b'|'
    ||| b == b'<'
    ||| b == b'>'
    ||| b == b'('
    ||| b == b')'
    ||| b == b'{'
    ||| b == b'}'
    ||| b == b'['
    ||| b == b']'
    ||| b == b'*'
    ||| b == b'?'
    ||| b == b'#'
    ||| b == b'\\'
    ||| b == b'!'
    ||| b == b'~'
    ||| b == b'%'
    ||| b == b'^'
}

/// Word bytes are never metacharacters (in particular never a newline, CR,
/// tab, quote, `$`, backtick, `;`, `&`, `|`, or a glob character).
pub proof fn lemma_word_bytes_are_inert(b: u8)
    ensures
        is_word_byte(b) ==> !is_shell_metachar(b) && b < 0x80,
        env_name::is_env_tail(b) ==> is_word_byte(b),
{
}

/// Model lexer for a POSIX shell (bash, zsh) or fish command line: a line
/// made only of word bytes and spaces, with no empty word (no leading,
/// trailing, or doubled space), is one simple command whose words are the
/// space-separated fields. Any other line is outside the model (`None`).
pub open spec fn simple_words(line: Seq<u8>) -> Option<Seq<Seq<u8>>> {
    let words = render::split_on(line, b' ');
    if (forall|i: int| 0 <= i < line.len() ==> is_word_byte(#[trigger] line[i]) || line[i] == b' ')
        && (forall|j: int| 0 <= j < words.len() ==> #[trigger] words[j].len() > 0) {
        Some(words)
    } else {
        None
    }
}

/// Splits an assignment word `N=V` at its first `=`.
pub open spec fn split_assignment(word: Seq<u8>) -> Option<(Seq<u8>, Seq<u8>)> {
    let k = render::first_index(word, b'=');
    if k < word.len() {
        Some((word.subrange(0, k), word.subrange(k + 1, word.len() as int)))
    } else {
        None
    }
}

/// Model reading of a POSIX line: `export N=V` with a portable `N`.
pub open spec fn posix_assignment(line: Seq<u8>) -> Option<(Seq<u8>, Seq<u8>)> {
    match simple_words(line) {
        Some(words) => if words.len() == 2 && words[0] == word_export() {
            match split_assignment(words[1]) {
                Some((n, v)) => if env_name::is_portable_env_name(n) {
                    Some((n, v))
                } else {
                    None
                },
                None => None,
            }
        } else {
            None
        },
        None => None,
    }
}

/// Model reading of a fish line: `set -x N V` with a portable `N` and a
/// value that is not an option.
pub open spec fn fish_assignment(line: Seq<u8>) -> Option<(Seq<u8>, Seq<u8>)> {
    match simple_words(line) {
        Some(words) => if words.len() == 4 && words[0] == word_set() && words[1] == word_dash_x()
            && env_name::is_portable_env_name(words[2]) && words[3][0] != b'-' {
            Some((words[2], words[3]))
        } else {
            None
        },
        None => None,
    }
}

/// Model reading of a `PowerShell` line: `$env:N="B"` with a portable `N`
/// (ending at the first `=`) and a double-quoted body `B` of word bytes, so
/// the body has no `"`, `` ` `` or `$` and is read literally.
pub open spec fn ps_assignment(line: Seq<u8>) -> Option<(Seq<u8>, Seq<u8>)> {
    if line.len() >= 8 && line.subrange(0, 5) == lit_env_prefix() && line.last() == b'"' {
        let body = line.subrange(5, line.len() - 1);
        let k = render::first_index(body, b'=');
        if k + 1 < body.len() && body[k + 1] == b'"' && env_name::is_portable_env_name(
            body.subrange(0, k),
        ) && (forall|i: int| k + 2 <= i < body.len() ==> is_word_byte(#[trigger] body[i])) {
            Some((body.subrange(0, k), body.subrange(k + 2, body.len() as int)))
        } else {
            None
        }
    } else {
        None
    }
}

/// The assignment a line performs in the given shell, under the model.
pub open spec fn parse_export_line(sh: ShellKind, line: Seq<u8>) -> Option<(Seq<u8>, Seq<u8>)> {
    match sh {
        ShellKind::Bash | ShellKind::Zsh => posix_assignment(line),
        ShellKind::Fish => fish_assignment(line),
        ShellKind::PowerShell => ps_assignment(line),
    }
}

/// Portable names and inert values are space-free, `=`-free (values may
/// not contain `=` either) and made of word bytes.
proof fn lemma_name_value_bytes(n: Seq<u8>, v: Seq<u8>)
    requires
        env_name::is_portable_env_name(n),
        is_inert_value(v),
    ensures
        render::free_of(n, b' '),
        render::free_of(n, b'='),
        render::free_of(v, b' '),
        render::free_of(v, b'='),
        forall|i: int| 0 <= i < n.len() ==> is_word_byte(#[trigger] n[i]),
        forall|i: int| 0 <= i < v.len() ==> is_word_byte(#[trigger] v[i]),
{
    env_name::lemma_portable_is_shell_inert(n);
    assert forall|i: int| 0 <= i < v.len() implies v[i] != b' ' && v[i] != b'=' by {
        assert(is_value_byte(v[i]));
    }
}

/// The words of the bash/zsh line are exactly `export` and `N=V`, and the
/// assignment word splits at its first `=` into exactly `N` and `V`.
pub proof fn lemma_posix_export_line(n: Seq<u8>, v: Seq<u8>)
    requires
        env_name::is_portable_env_name(n),
        is_inert_value(v),
    ensures
        simple_words(export_line_spec(ShellKind::Bash, n, v)) == Some(
            seq![word_export(), n + seq![b'='] + v],
        ),
        split_assignment(n + seq![b'='] + v) == Some((n, v)),
        posix_assignment(export_line_spec(ShellKind::Bash, n, v)) == Some((n, v)),
        export_line_spec(ShellKind::Bash, n, v) == export_line_spec(ShellKind::Zsh, n, v),
{
    lemma_name_value_bytes(n, v);
    let line = export_line_spec(ShellKind::Bash, n, v);
    let w = n + seq![b'='] + v;
    let parts = seq![word_export(), w];
    lemma_posix_line_join(n, v);
    lemma_assignment_word(n, v);
    assert(render::free_of(word_export(), b' '));
    assert forall|i: int| 0 <= i < parts.len() implies render::free_of(#[trigger] parts[i], b' ') by {
        if i == 0 {
            assert(parts[i] == word_export());
        } else {
            assert(parts[i] == w);
        }
    }
    render::lemma_split_join(parts, b' ');
    lemma_posix_line_bytes(n, v);
    let words = render::split_on(line, b' ');
    assert forall|j: int| 0 <= j < words.len() implies #[trigger] words[j].len() > 0 by {
        assert(words[j] == parts[j]);
    }
}

/// The assignment word `N=V` is space-free and splits at its first `=`
/// into exactly `N` and `V`.
proof fn lemma_assignment_word(n: Seq<u8>, v: Seq<u8>)
    requires
        render::free_of(n, b' '),
        render::free_of(n, b'='),
        render::free_of(v, b' '),
    ensures
        render::free_of(n + seq![b'='] + v, b' '),
        split_assignment(n + seq![b'='] + v) == Some((n, v)),
{
    let w = n + seq![b'='] + v;
    assert forall|i: int| 0 <= i < w.len() implies w[i] != b' ' by {
        if i < n.len() {
            assert(w[i] == n[i]);
        } else if i > n.len() {
            assert(w[i] == v[i - n.len() - 1]);
        }
    }
    render::lemma_first_index_at(n, b'=', v);
    assert(n.push(b'=') + v =~= w);
    assert(w.subrange(0, n.len() as int) =~= n);
    assert(w.subrange(n.len() as int + 1, w.len() as int) =~= v);
}

/// The bash/zsh line is the space-join of `export` and `N=V`.
proof fn lemma_posix_line_join(n: Seq<u8>, v: Seq<u8>)
    ensures
        export_line_spec(ShellKind::Bash, n, v) == render::join_with(
            seq![word_export(), n + seq![b'='] + v],
            b' ',
        ),
        export_line_spec(ShellKind::Bash, n, v) == export_line_spec(ShellKind::Zsh, n, v),
{
    let w = n + seq![b'='] + v;
    let p1 = seq![word_export()];
    let p2 = p1.push(w);
    assert(p2 =~= seq![word_export(), w]);
    assert(render::join_with(p1, b' ') == word_export());
    assert(p2.drop_last() =~= p1);
    assert(render::join_with(p2, b' ') == word_export().push(b' ') + w);
    assert(export_line_spec(ShellKind::Bash, n, v) =~= word_export().push(b' ') + w);
}

/// Every byte of the bash/zsh line is a word byte or a space.
proof fn lemma_posix_line_bytes(n: Seq<u8>, v: Seq<u8>)
    requires
        forall|i: int| 0 <= i < n.len() ==> is_word_byte(#[trigger] n[i]),
        forall|i: int| 0 <= i < v.len() ==> is_word_byte(#[trigger] v[i]),
    ensures
        forall|i: int|
            0 <= i < export_line_spec(ShellKind::Bash, n, v).len() ==> is_word_byte(
                #[trigger] export_line_spec(ShellKind::Bash, n, v)[i],
            ) || export_line_spec(ShellKind::Bash, n, v)[i] == b' ',
{
    let line = export_line_spec(ShellKind::Bash, n, v);
    assert forall|i: int| 0 <= i < line.len() implies is_word_byte(#[trigger] line[i]) || line[i]
        == b' ' by {
        if i < 7 {
            assert(line[i] == lit_export_sp()[i]);
        } else if i < 7 + n.len() {
            assert(line[i] == n[i - 7]);
        } else if i > 7 + n.len() {
            assert(line[i] == v[i - 8 - n.len()]);
        }
    }
}

/// The words of the fish line are exactly `set`, `-x`, `N` and `V`.
pub proof fn lemma_fish_export_line(n: Seq<u8>, v: Seq<u8>)
    requires
        env_name::is_portable_env_name(n),
        is_inert_value(v),
    ensures
        simple_words(export_line_spec(ShellKind::Fish, n, v)) == Some(
            seq![word_set(), word_dash_x(), n, v],
        ),
        fish_assignment(export_line_spec(ShellKind::Fish, n, v)) == Some((n, v)),
{
    lemma_name_value_bytes(n, v);
    let line = export_line_spec(ShellKind::Fish, n, v);
    let parts = seq![word_set(), word_dash_x(), n, v];
    lemma_fish_line_join(n, v);
    assert(render::free_of(word_set(), b' '));
    assert(render::free_of(word_dash_x(), b' '));
    assert forall|i: int| 0 <= i < parts.len() implies render::free_of(#[trigger] parts[i], b' ') by {
        if i == 0 {
            assert(parts[i] == word_set());
        } else if i == 1 {
            assert(parts[i] == word_dash_x());
        } else if i == 2 {
            assert(parts[i] == n);
        } else {
            assert(parts[i] == v);
        }
    }
    render::lemma_split_join(parts, b' ');
    lemma_fish_line_bytes(n, v);
    let words = render::split_on(line, b' ');
    assert forall|j: int| 0 <= j < words.len() implies #[trigger] words[j].len() > 0 by {
        assert(words[j] == parts[j]);
    }
}

/// The fish line is the space-join of `set`, `-x`, `N` and `V`.
proof fn lemma_fish_line_join(n: Seq<u8>, v: Seq<u8>)
    ensures
        export_line_spec(ShellKind::Fish, n, v) == render::join_with(
            seq![word_set(), word_dash_x(), n, v],
            b' ',
        ),
{
    let p1 = seq![word_set()];
    let p2 = p1.push(word_dash_x());
    let p3 = p2.push(n);
    let p4 = p3.push(v);
    assert(p4 =~= seq![word_set(), word_dash_x(), n, v]);
    assert(render::join_with(p1, b' ') == word_set());
    assert(p2.drop_last() =~= p1);
    assert(p3.drop_last() =~= p2);
    assert(p4.drop_last() =~= p3);
    let j2 = word_set().push(b' ') + word_dash_x();
    let j3 = j2.push(b' ') + n;
    let j4 = j3.push(b' ') + v;
    assert(render::join_with(p2, b' ') == j2);
    assert(render::join_with(p3, b' ') == j3);
    assert(render::join_with(p4, b' ') == j4);
    assert(j2 =~= lit_set_x_sp().drop_last());
    assert(export_line_spec(ShellKind::Fish, n, v) =~= j4);
}

/// Every byte of the fish line is a word byte or a space.
proof fn lemma_fish_line_bytes(n: Seq<u8>, v: Seq<u8>)
    requires
        forall|i: int| 0 <= i < n.len() ==> is_word_byte(#[trigger] n[i]),
        forall|i: int| 0 <= i < v.len() ==> is_word_byte(#[trigger] v[i]),
    ensures
        forall|i: int|
            0 <= i < export_line_spec(ShellKind::Fish, n, v).len() ==> is_word_byte(
                #[trigger] export_line_spec(ShellKind::Fish, n, v)[i],
            ) || export_line_spec(ShellKind::Fish, n, v)[i] == b' ',
{
    let line = export_line_spec(ShellKind::Fish, n, v);
    assert forall|i: int| 0 <= i < line.len() implies is_word_byte(#[trigger] line[i]) || line[i]
        == b' ' by {
        if i < 7 {
            assert(line[i] == lit_set_x_sp()[i]);
        } else if i < 7 + n.len() {
            assert(line[i] == n[i - 7]);
        } else if i > 7 + n.len() {
            assert(line[i] == v[i - 8 - n.len()]);
        }
    }
}

/// The `PowerShell` line is read as the assignment of exactly `V` to `N`,
/// and its only bytes outside word bytes are `$` at 0 and the two quotes.
pub proof fn lemma_ps_export_line(n: Seq<u8>, v: Seq<u8>)
    requires
        env_name::is_portable_env_name(n),
        is_inert_value(v),
    ensures
        ps_assignment(export_line_spec(ShellKind::PowerShell, n, v)) == Some((n, v)),
        forall|i: int|
            0 <= i < export_line_spec(ShellKind::PowerShell, n, v).len() ==> {
                let b = #[trigger] export_line_spec(ShellKind::PowerShell, n, v)[i];
                is_word_byte(b) || (i == 0 && b == b'$') || (b == b'"' && (i == n.len() + 6 || i
                    == n.len() + v.len() + 7))
            },
{
    lemma_name_value_bytes(n, v);
    let line = export_line_spec(ShellKind::PowerShell, n, v);
    assert(line.len() == n.len() + v.len() + 8);
    assert(line.subrange(0, 5) =~= lit_env_prefix());
    let body = line.subrange(5, line.len() - 1);
    assert(body =~= n.push(b'=') + (seq![b'"'] + v));
    render::lemma_first_index_at(n, b'=', seq![b'"'] + v);
    assert(body.subrange(0, n.len() as int) =~= n);
    assert(body.subrange(n.len() as int + 2, body.len() as int) =~= v);
    assert forall|i: int| n.len() + 2 <= i < body.len() implies is_word_byte(#[trigger] body[i]) by {
        assert(body[i] == v[i - n.len() - 2]);
    }
    assert forall|i: int| 0 <= i < line.len() implies {
        let b = #[trigger] line[i];
        is_word_byte(b) || (i == 0 && b == b'$') || (b == b'"' && (i == n.len() + 6 || i == n.len()
            + v.len() + 7))
    } by {
        if 5 <= i < 5 + n.len() {
            assert(line[i] == n[i - 5]);
        } else if n.len() + 7 <= i < n.len() + v.len() + 7 {
            assert(line[i] == v[i - n.len() - 7]);
        }
    }
}

/// Round trip: every shell reads the line as the assignment of exactly `V`
/// to `N`.
pub proof fn lemma_export_round_trip(sh: ShellKind, n: Seq<u8>, v: Seq<u8>)
    requires
        env_name::is_portable_env_name(n),
        is_inert_value(v),
    ensures
        parse_export_line(sh, export_line_spec(sh, n, v)) == Some((n, v)),
{
    match sh {
        ShellKind::Bash | ShellKind::Zsh => lemma_posix_export_line(n, v),
        ShellKind::Fish => lemma_fish_export_line(n, v),
        ShellKind::PowerShell => lemma_ps_export_line(n, v),
    }
}

/// Injectivity: different assignments give different lines.
pub proof fn lemma_export_line_injective(
    sh: ShellKind,
    n1: Seq<u8>,
    v1: Seq<u8>,
    n2: Seq<u8>,
    v2: Seq<u8>,
)
    requires
        env_name::is_portable_env_name(n1),
        is_inert_value(v1),
        env_name::is_portable_env_name(n2),
        is_inert_value(v2),
        export_line_spec(sh, n1, v1) == export_line_spec(sh, n2, v2),
    ensures
        n1 == n2,
        v1 == v2,
{
    lemma_export_round_trip(sh, n1, v1);
    lemma_export_round_trip(sh, n2, v2);
}

/// Every byte of an export line is ASCII, and the line has no newline or
/// carriage return.
pub proof fn lemma_export_line_ascii(sh: ShellKind, n: Seq<u8>, v: Seq<u8>)
    requires
        env_name::is_portable_env_name(n),
        is_inert_value(v),
    ensures
        text::is_ascii_bytes(export_line_spec(sh, n, v)),
        render::free_of(export_line_spec(sh, n, v), b'\n'),
        render::free_of(export_line_spec(sh, n, v), b'\r'),
        export_line_spec(sh, n, v).len() > 0,
{
    let line = export_line_spec(sh, n, v);
    match sh {
        ShellKind::Bash | ShellKind::Zsh => lemma_posix_export_line(n, v),
        ShellKind::Fish => lemma_fish_export_line(n, v),
        ShellKind::PowerShell => lemma_ps_export_line(n, v),
    }
    assert forall|i: int| 0 <= i < line.len() implies #[trigger] line[i] < 0x80 && line[i] != b'\n'
        && line[i] != b'\r' by {
        lemma_word_bytes_are_inert(line[i]);
    }
}

// ---------------------------------------------------------------------------
// Executable functions
// ---------------------------------------------------------------------------

/// Whether `s[lo..hi]` equals `lit` ignoring ASCII case (false when the
/// range is out of bounds).
fn ci_range(s: &[u8], lo: usize, hi: usize, lit: &[u8]) -> (r: bool)
    ensures
        r == (lo <= hi <= s.len() && ci(s@.subrange(lo as int, hi as int), lit@)),
{
    if lo > hi || hi > s.len() || hi - lo != lit.len() {
        return false;
    }
    let mut i: usize = 0;
    while i < lit.len()
        invariant
            lo <= hi <= s.len(),
            hi - lo == lit.len(),
            0 <= i <= lit.len(),
            forall|j: int|
                0 <= j < i ==> text::ascii_upper(#[trigger] s@[lo + j]) == text::ascii_upper(
                    lit@[j],
                ),
        decreases lit.len() - i,
    {
        if text::upper_byte(s[lo + i]) != text::upper_byte(lit[i]) {
            proof {
                assert(s@.subrange(lo as int, hi as int)[i as int] == s@[lo + i]);
            }
            return false;
        }
        i += 1;
    }
    proof {
        let sub = s@.subrange(lo as int, hi as int);
        assert forall|j: int| 0 <= j < sub.len() implies text::ascii_upper(#[trigger] sub[j])
            == text::ascii_upper(lit@[j]) by {
            assert(sub[j] == s@[lo + j]);
        }
    }
    true
}

/// Parses a `--shell` value (see `shell_alias`).
#[allow(clippy::byte_char_slices)]
pub fn parse_shell_name(s: &[u8]) -> (r: Option<ShellKind>)
    ensures
        r == shell_alias(s@),
{
    let n = s.len();
    let bash = [b'b', b'a', b's', b'h'];
    let sh = [b's', b'h'];
    let zsh = [b'z', b's', b'h'];
    let fish = [b'f', b'i', b's', b'h'];
    let powershell = [b'p', b'o', b'w', b'e', b'r', b's', b'h', b'e', b'l', b'l'];
    let pwsh = [b'p', b'w', b's', b'h'];
    let ps = [b'p', b's'];
    proof {
        assert(bash@ =~= lit_bash());
        assert(sh@ =~= lit_sh());
        assert(zsh@ =~= lit_zsh());
        assert(fish@ =~= lit_fish());
        assert(powershell@ =~= lit_powershell());
        assert(pwsh@ =~= lit_pwsh());
        assert(ps@ =~= lit_ps());
        assert(s@.subrange(0, n as int) =~= s@);
    }
    if ci_range(s, 0, n, bash.as_slice()) || ci_range(s, 0, n, sh.as_slice()) {
        Some(ShellKind::Bash)
    } else if ci_range(s, 0, n, zsh.as_slice()) {
        Some(ShellKind::Zsh)
    } else if ci_range(s, 0, n, fish.as_slice()) {
        Some(ShellKind::Fish)
    } else if ci_range(s, 0, n, powershell.as_slice()) || ci_range(s, 0, n, pwsh.as_slice())
        || ci_range(s, 0, n, ps.as_slice()) {
        Some(ShellKind::PowerShell)
    } else {
        None
    }
}

/// Where the basename of `p` starts.
fn basename_start_index(p: &[u8]) -> (start: usize)
    ensures
        start == basename_start(p@),
{
    let mut i = p.len();
    while i > 0 && p[i - 1] != b'/' && p[i - 1] != b'\\'
        invariant
            0 <= i <= p.len(),
            basename_start(p@) == basename_start(p@.subrange(0, i as int)),
        decreases i,
    {
        proof {
            assert(p@.subrange(0, i as int).drop_last() =~= p@.subrange(0, i - 1));
        }
        i -= 1;
    }
    proof {
        assert(p@.subrange(0, p@.len() as int) =~= p@);
        if i > 0 {
            assert(p@.subrange(0, i as int).last() == p@[i - 1]);
        }
    }
    i
}

/// Classifies a `SHELL` value by its basename (see
/// `classify_shell_path_spec`).
#[allow(clippy::byte_char_slices)]
pub fn classify_shell_path(p: &[u8]) -> (r: Option<ShellKind>)
    ensures
        r == classify_shell_path_spec(p@),
{
    let len = p.len();
    let start = basename_start_index(p);
    proof {
        lemma_basename_shape(p@);
    }
    let exe = [b'.', b'e', b'x', b'e'];
    proof {
        assert(exe@ =~= lit_exe());
    }
    let end = if len - start >= 4 && ci_range(p, len - 4, len, exe.as_slice()) {
        len - 4
    } else {
        len
    };
    let ghost name = basename(p@);
    proof {
        if len - start >= 4 {
            assert(name.subrange(name.len() - 4, name.len() as int) =~= p@.subrange(
                len - 4,
                len as int,
            ));
        }
        assert(program_stem(name) =~= p@.subrange(start as int, end as int));
    }
    let zsh = [b'z', b's', b'h'];
    let fish = [b'f', b'i', b's', b'h'];
    let pwsh = [b'p', b'w', b's', b'h'];
    let powershell = [b'p', b'o', b'w', b'e', b'r', b's', b'h', b'e', b'l', b'l'];
    let bash = [b'b', b'a', b's', b'h'];
    let sh = [b's', b'h'];
    let dash = [b'd', b'a', b's', b'h'];
    let ksh = [b'k', b's', b'h'];
    let mksh = [b'm', b'k', b's', b'h'];
    let ash = [b'a', b's', b'h'];
    let yash = [b'y', b'a', b's', b'h'];
    proof {
        assert(zsh@ =~= lit_zsh());
        assert(fish@ =~= lit_fish());
        assert(pwsh@ =~= lit_pwsh());
        assert(powershell@ =~= lit_powershell());
        assert(bash@ =~= lit_bash());
        assert(sh@ =~= lit_sh());
        assert(dash@ =~= lit_dash());
        assert(ksh@ =~= lit_ksh());
        assert(mksh@ =~= lit_mksh());
        assert(ash@ =~= lit_ash());
        assert(yash@ =~= lit_yash());
    }
    if ci_range(p, start, end, zsh.as_slice()) {
        Some(ShellKind::Zsh)
    } else if ci_range(p, start, end, fish.as_slice()) {
        Some(ShellKind::Fish)
    } else if ci_range(p, start, end, pwsh.as_slice()) || ci_range(
        p,
        start,
        end,
        powershell.as_slice(),
    ) {
        Some(ShellKind::PowerShell)
    } else if ci_range(p, start, end, bash.as_slice()) || ci_range(p, start, end, sh.as_slice())
        || ci_range(p, start, end, dash.as_slice()) || ci_range(p, start, end, ksh.as_slice())
        || ci_range(p, start, end, mksh.as_slice()) || ci_range(p, start, end, ash.as_slice())
        || ci_range(p, start, end, yash.as_slice()) {
        Some(ShellKind::Bash)
    } else {
        None
    }
}

/// Detects the shell (see `detected_shell`).
pub fn detect_shell(signals: ShellSignals, shell_var: Option<&[u8]>) -> (r: Option<ShellKind>)
    ensures
        r == detected_shell(
            signals,
            match shell_var {
                Some(p) => Some(p@),
                None => None,
            },
        ),
{
    if signals.zsh_version {
        Some(ShellKind::Zsh)
    } else if signals.fish_version {
        Some(ShellKind::Fish)
    } else if signals.ps_module_path {
        Some(ShellKind::PowerShell)
    } else {
        match shell_var {
            Some(p) => classify_shell_path(p),
            None => None,
        }
    }
}

/// Whether `b` may appear in an exported value.
const fn value_byte(b: u8) -> (r: bool)
    ensures
        r == is_value_byte(b),
{
    matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'/' | b':' | b'@' | b'+' | b',' | b'-')
}

/// Whether `v` is an inert value (see `is_inert_value`).
pub fn is_inert_value_bytes(v: &[u8]) -> (r: bool)
    ensures
        r == is_inert_value(v@),
{
    if v.is_empty() || v[0] == b'-' {
        return false;
    }
    let mut i: usize = 0;
    while i < v.len()
        invariant
            0 <= i <= v.len(),
            forall|j: int| 0 <= j < i ==> #[trigger] is_value_byte(v@[j]),
        decreases v.len() - i,
    {
        if !value_byte(v[i]) {
            return false;
        }
        i += 1;
    }
    true
}

/// Appends the export line for `name` and `value` (see `export_line_spec`).
#[allow(clippy::byte_char_slices)]
pub fn push_export_line(out: &mut Vec<u8>, sh: ShellKind, name: &[u8], value: &[u8])
    ensures
        final(out)@ == old(out)@ + export_line_spec(sh, name@, value@),
{
    let ghost start = out@;
    match sh {
        ShellKind::Bash | ShellKind::Zsh => {
            let prefix = [b'e', b'x', b'p', b'o', b'r', b't', b' '];
            proof {
                assert(prefix@ =~= lit_export_sp());
            }
            render::push_bytes(out, prefix.as_slice());
            render::push_bytes(out, name);
            out.push(b'=');
            render::push_bytes(out, value);
            proof {
                assert(out@ =~= start + export_line_spec(sh, name@, value@));
            };
        },
        ShellKind::Fish => {
            let prefix = [b's', b'e', b't', b' ', b'-', b'x', b' '];
            proof {
                assert(prefix@ =~= lit_set_x_sp());
            }
            render::push_bytes(out, prefix.as_slice());
            render::push_bytes(out, name);
            out.push(b' ');
            render::push_bytes(out, value);
            proof {
                assert(out@ =~= start + export_line_spec(sh, name@, value@));
            };
        },
        ShellKind::PowerShell => {
            let prefix = [b'$', b'e', b'n', b'v', b':'];
            proof {
                assert(prefix@ =~= lit_env_prefix());
            }
            render::push_bytes(out, prefix.as_slice());
            render::push_bytes(out, name);
            out.push(b'=');
            out.push(b'"');
            render::push_bytes(out, value);
            out.push(b'"');
            proof {
                assert(out@ =~= start + export_line_spec(sh, name@, value@));
            };
        },
    }
}

/// The export line for an explicit name and value: `Some` exactly when the
/// name is portable and the value is inert, and then the line of
/// `export_line_spec` (as text; the line is ASCII).
pub fn export_line(sh: ShellKind, name: &str, value: &str) -> (r: Option<String>)
    ensures
        r is Some <==> env_name::is_portable_env_name(vstd::utf8::encode_utf8(name@))
            && is_inert_value(vstd::utf8::encode_utf8(value@)),
        r matches Some(line) ==> vstd::utf8::encode_utf8(line@) == export_line_spec(
            sh,
            vstd::utf8::encode_utf8(name@),
            vstd::utf8::encode_utf8(value@),
        ),
{
    let name_bytes = name.as_bytes();
    let value_bytes = value.as_bytes();
    if env_name::check_env_name_bytes(name_bytes).is_err() || !is_inert_value_bytes(value_bytes) {
        return None;
    }
    let mut line: Vec<u8> = Vec::new();
    push_export_line(&mut line, sh, name_bytes, value_bytes);
    proof {
        assert(line@ =~= export_line_spec(sh, name_bytes@, value_bytes@));
        lemma_export_line_ascii(sh, name_bytes@, value_bytes@);
        text::lemma_ascii_bytes_round_trip(line@);
    }
    Some(text::string_from_ascii(line.as_slice()))
}

} // verus!
