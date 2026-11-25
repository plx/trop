//! Verified kernel: project and task name derivation from repository facts.
//!
//! Git discovery, canonicalization, and HEAD lookup stay in the production
//! glue (`crate::operations::inference`). The kernel decides the pure string
//! steps on the UTF-8 bytes of the names the glue found:
//!
//! - the project is the repository directory's name — the parent of the
//!   common directory when that is `.git`, otherwise the common directory
//!   itself (a bare repository) — with one trailing `.git` removed
//!   (`/path/to/repo.git` names project `repo`);
//! - the task of a non-worktree checkout is the current branch: the HEAD
//!   reference with its `refs/heads/` prefix removed (slashes inside the
//!   branch name are kept); any other reference names no branch.
//!
//! The kernel returns byte lengths and offsets; they always fall next to an
//! ASCII byte (or at an end), so they are UTF-8 character boundaries of the
//! glue's `&str` and slicing there cannot split a character.

use vstd::prelude::*;

verus! {

// ---------------------------------------------------------------------------
// Specifications
// ---------------------------------------------------------------------------

/// The bytes of `.git`.
pub open spec fn git_suffix() -> Seq<u8> {
    seq![0x2eu8, 0x67u8, 0x69u8, 0x74u8]
}

/// The bytes of `refs/heads/`.
pub open spec fn branch_ref_prefix() -> Seq<u8> {
    seq![
        0x72u8,
        0x65u8,
        0x66u8,
        0x73u8,
        0x2fu8,
        0x68u8,
        0x65u8,
        0x61u8,
        0x64u8,
        0x73u8,
        0x2fu8,
    ]
}

/// `s` ends with `suffix`.
pub open spec fn ends_with(s: Seq<u8>, suffix: Seq<u8>) -> bool {
    suffix.len() <= s.len() && s.subrange(s.len() - suffix.len(), s.len() as int) == suffix
}

/// `s` starts with `prefix`.
pub open spec fn starts_with(s: Seq<u8>, prefix: Seq<u8>) -> bool {
    prefix.len() <= s.len() && s.subrange(0, prefix.len() as int) == prefix
}

/// `name` with one trailing `.git` removed, if it has one.
pub open spec fn strip_git_suffix(name: Seq<u8>) -> Seq<u8> {
    if ends_with(name, git_suffix()) {
        name.subrange(0, name.len() - 4)
    } else {
        name
    }
}

/// The repository name derived from the canonical common directory's last
/// component (`common`) and its parent's (`parent`, if any and valid
/// UTF-8): the parent when `common` is `.git`, otherwise `common` itself
/// (a bare repository), with one `.git` suffix stripped.
pub open spec fn repository_name(common: Seq<u8>, parent: Option<Seq<u8>>) -> Option<Seq<u8>> {
    if common == git_suffix() {
        match parent {
            Some(name) => Some(strip_git_suffix(name)),
            None => None,
        }
    } else {
        Some(strip_git_suffix(common))
    }
}

/// The branch a HEAD reference names: the reference without `refs/heads/`,
/// or none for any other reference.
pub open spec fn branch_name(head_ref: Seq<u8>) -> Option<Seq<u8>> {
    if starts_with(head_ref, branch_ref_prefix()) {
        Some(head_ref.subrange(11, head_ref.len() as int))
    } else {
        None
    }
}

/// Offset `i` is next to an ASCII byte of `s` (or at an end of `s`), so it
/// is a UTF-8 character boundary of any valid UTF-8 `s`: an ASCII byte is a
/// whole character.
pub open spec fn ascii_boundary(s: Seq<u8>, i: int) -> bool {
    &&& 0 <= i <= s.len()
    &&& (i == 0 || i == s.len() || s[i] < 0x80 || s[i - 1] < 0x80)
}

// ---------------------------------------------------------------------------
// Executable kernels
// ---------------------------------------------------------------------------

/// Whose name names the repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameSource {
    /// The common directory's own name (a bare repository).
    CommonDirectory,
    /// The common directory's parent's name (the common directory is
    /// `.git`).
    ParentDirectory,
}

/// A repository name: the first `len` bytes of the name `source` selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepositoryName {
    /// The name the repository name is a prefix of.
    pub source: NameSource,
    /// The length of the repository name, in bytes.
    pub len: usize,
}

/// The bytes a name source selects.
pub open spec fn selected(source: NameSource, common: Seq<u8>, parent: Option<Seq<u8>>) -> Seq<u8> {
    match source {
        NameSource::CommonDirectory => common,
        NameSource::ParentDirectory => match parent {
            Some(name) => name,
            None => Seq::empty(),
        },
    }
}

/// The contents of an optional byte string.
pub open spec fn opt_bytes(value: Option<&[u8]>) -> Option<Seq<u8>> {
    match value {
        Some(bytes) => Some(bytes@),
        None => None,
    }
}

/// `name` is the bytes of `.git`.
fn is_git_suffix(name: &[u8]) -> (r: bool)
    ensures
        r == (name@ == git_suffix()),
{
    if name.len() != 4 {
        return false;
    }
    let r = name[0] == 0x2e && name[1] == 0x67 && name[2] == 0x69 && name[3] == 0x74;
    proof {
        if r {
            assert(name@ =~= git_suffix());
        }
    }
    r
}

/// The length of `name` with one trailing `.git` removed (see
/// `strip_git_suffix`). The result is an ASCII boundary of `name`.
pub fn git_suffix_stripped_len(name: &[u8]) -> (n: usize)
    ensures
        n <= name@.len(),
        name@.subrange(0, n as int) == strip_git_suffix(name@),
        ascii_boundary(name@, n as int),
{
    let len = name.len();
    if len < 4 {
        assert(name@.subrange(0, len as int) =~= name@);
        return len;
    }
    let start = len - 4;
    let has_suffix = name[start] == 0x2e && name[start + 1] == 0x67 && name[start + 2] == 0x69
        && name[start + 3] == 0x74;
    let ghost tail = name@.subrange(start as int, len as int);
    if has_suffix {
        assert(tail =~= git_suffix());
        start
    } else {
        assert(tail != git_suffix()) by {
            if tail == git_suffix() {
                assert(tail[0] == name@[start as int]);
                assert(tail[1] == name@[start as int + 1]);
                assert(tail[2] == name@[start as int + 2]);
                assert(tail[3] == name@[start as int + 3]);
            }
        }
        assert(name@.subrange(0, len as int) =~= name@);
        len
    }
}

/// Derives the repository name (see `repository_name`): which component
/// names the repository, and how many of its bytes do. `None` exactly when
/// the common directory is `.git` and its parent has no usable name.
pub fn repository_name_prefix(common: &[u8], parent: Option<&[u8]>) -> (r: Option<RepositoryName>)
    ensures
        r is Some <==> repository_name(common@, opt_bytes(parent)) is Some,
        r matches Some(name) ==> {
            let chosen = selected(name.source, common@, opt_bytes(parent));
            &&& (name.source == NameSource::ParentDirectory <==> common@ == git_suffix())
            &&& name.source == NameSource::ParentDirectory ==> parent is Some
            &&& name.len <= chosen.len()
            &&& chosen.subrange(0, name.len as int) == repository_name(
                common@,
                opt_bytes(parent),
            )->Some_0
            &&& ascii_boundary(chosen, name.len as int)
        },
{
    if is_git_suffix(common) {
        let name = parent?;
        Some(RepositoryName { source: NameSource::ParentDirectory, len: git_suffix_stripped_len(name) })
    } else {
        Some(
            RepositoryName {
                source: NameSource::CommonDirectory,
                len: git_suffix_stripped_len(common),
            },
        )
    }
}

/// Where the branch name starts in a HEAD reference (see `branch_name`):
/// `Some(11)` exactly for references under `refs/heads/`, `None` for any
/// other reference. The offset is an ASCII boundary.
pub fn branch_name_start(head_ref: &[u8]) -> (r: Option<usize>)
    ensures
        r is Some <==> branch_name(head_ref@) is Some,
        r matches Some(start) ==> start == 11 && start <= head_ref@.len() && head_ref@.subrange(
            start as int,
            head_ref@.len() as int,
        ) == branch_name(head_ref@)->Some_0 && ascii_boundary(head_ref@, start as int),
{
    if head_ref.len() < 11 {
        return None;
    }
    let prefix: [u8; 11] = [0x72, 0x65, 0x66, 0x73, 0x2f, 0x68, 0x65, 0x61, 0x64, 0x73, 0x2f];
    let ghost expected = branch_ref_prefix();
    assert(prefix@ =~= expected);
    let mut i: usize = 0;
    while i < 11
        invariant
            head_ref@.len() >= 11,
            expected == branch_ref_prefix(),
            prefix@ == expected,
            0 <= i <= 11,
            forall|j: int| 0 <= j < i ==> #[trigger] head_ref@[j] == expected[j],
        decreases 11 - i,
    {
        if head_ref[i] != prefix[i] {
            proof {
                assert(prefix@[i as int] == expected[i as int]);
                assert(head_ref@.subrange(0, 11)[i as int] == head_ref@[i as int]);
                assert(head_ref@.subrange(0, 11) != expected);
            }
            return None;
        }
        i += 1;
    }
    assert(head_ref@.subrange(0, 11) =~= expected);
    Some(11)
}

} // verus!
