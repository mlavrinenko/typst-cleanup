//! Consumer-blind orchestration for retiring Typst artifacts.
//!
//! The shared cleanup skeleton across file-based trackers: refuse a dirty tree,
//! mint a `swh:1:rev` tombstone per retired file ([`swhid_mint`]), rewrite inbound
//! references in survivors ([`typst_edit`]) onto those tombstones, and delete the
//! retired files — without committing, so the operator reviews and commits.
//!
//! Atomic by construction: [`plan`] mints every tombstone and computes every edit
//! in memory; not a byte is written until [`apply`] runs on a complete plan. A
//! wrong-shaped file or an unrecoverable commit aborts the run with no changes,
//! and [`Error::Ineligible`] names every file that can't carry a tombstone.
//!
//! Git is needed only when a survivor references a retired artifact.
//! [`referrers`] says which survivors do, with no git; when none do,
//! [`delete_plan`] retires the artifacts by deleting them.
//!
//! The crate knows nothing about what an artifact *is*, nor how its inbound edges
//! are spelled. The consumer supplies the policy via [`Policy`]: where the
//! rewritable references live ([`Policy::scan`] — `link("…")` calls by default,
//! overridable for a DSL's own edge helpers), which of them name artifacts
//! ([`Policy::identity`]), and how a retired artifact's tombstone reads
//! ([`Policy::tombstone`]). It also partitions its artifacts into the retired
//! ([`Artifact`]s passed as `eligible`) and the survivors to scan.

mod error;
mod git;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};

pub use error::{Error, Ineligible, Reason};
pub use git::{is_clean, toplevel};
// Re-exported because it appears in `Policy::tombstone`'s signature: consumers
// implementing the trait need to name the minted history pointer's type.
pub use swhid_mint::Swhid;

/// An artifact on disk: a file cleanup may retire, or scan for inbound links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// Absolute path to the file.
    pub abs: PathBuf,
    /// Display path (root- or repository-relative), used in reports.
    pub rel: PathBuf,
    /// Identity a `link()` target resolves to (e.g. the file stem).
    pub id: String,
}

/// A rewritable reference located in a survivor's source: the raw target string
/// as authored and the byte range to splice.
///
/// Returned by [`Policy::scan`] and fed straight to [`typst_edit::Edit`] — the
/// range covers the quoted string literal, quotes included, for a string target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The target string as authored, unquoted and unescaped (e.g. `"other.typ"`).
    pub raw: String,
    /// Byte range of the reference in the source, ready for [`typst_edit::Edit`].
    pub range: Range<usize>,
}

/// Consumer policy: where a survivor's rewritable references are, which of them
/// name artifacts, and how a retired artifact's tombstone reads. Every method is
/// consumer-supplied — the crate bakes in no reference shape, identity scheme, or
/// label format.
pub trait Policy {
    /// Locate every rewritable reference in a survivor's `source`.
    ///
    /// The default covers `link("…")` calls — the thin, common case. Override it
    /// to scan a DSL's own edge helpers (`depends-on("x.typ")`, `edge(kind, …)`,
    /// …) or any other reference shape; the crate keeps owning the mint, the
    /// dirty-tree guard, and the atomic plan/apply around it.
    fn scan(&self, source: &str) -> Vec<Target> {
        typst_edit::find_link_targets(source)
            .into_iter()
            .map(|found| Target {
                raw: found.url,
                range: found.range,
            })
            .collect()
    }

    /// Map a raw target (from [`Policy::scan`]) to the artifact identity it
    /// names, or `None` when it is not an inbound artifact edge — an external
    /// URL, or an existing tombstone that must be left untouched.
    fn identity(&self, raw: &str) -> Option<String>;

    /// Replacement text for a retired artifact's inbound reference: the quoted
    /// SWHID tombstone, optionally carrying a synthesized label. Called once per
    /// retired artifact; `swhid` is its freshly minted history pointer, whose
    /// path is `/`-separated on every platform.
    fn tombstone(&self, retired: &Artifact, swhid: &Swhid) -> String;
}

/// A file the plan will delete: its absolute path and display path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deletion {
    /// Absolute path to delete.
    pub abs: PathBuf,
    /// Display path for reports.
    pub rel: PathBuf,
}

/// A surviving file the plan will rewrite, with its already-spliced contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    /// Absolute path to overwrite.
    pub abs: PathBuf,
    /// Display path for reports.
    pub rel: PathBuf,
    /// The rewritten file contents.
    pub contents: String,
    /// How many links were retargeted in this file.
    pub links: usize,
}

/// The full, computed-in-memory cleanup. Empty when nothing is eligible.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Plan {
    /// Files to delete.
    pub deletions: Vec<Deletion>,
    /// Surviving files to rewrite.
    pub rewrites: Vec<Rewrite>,
}

impl Plan {
    /// Whether the plan deletes nothing (and so rewrites nothing).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.deletions.is_empty()
    }
}

/// Compute the whole cleanup without touching disk.
///
/// Mints a tombstone for every `eligible` artifact and splices every
/// `survivor`'s inbound references onto those tombstones. `git_root` is the
/// repository root every eligible path must lie within. Returns an empty
/// [`Plan`] when `eligible` is empty. When no survivor references a retired
/// artifact, [`delete_plan`] retires them without git; [`referrers`] says which.
///
/// # Errors
/// [`Error::Ineligible`] naming every artifact outside the repository,
/// untracked, or diverging from git; else a survivor read failure or a
/// rejected splice. Each comes before any byte is written.
pub fn plan<P: Policy>(
    eligible: &[Artifact],
    survivors: &[Artifact],
    git_root: &Path,
    policy: &P,
) -> Result<Plan, Error> {
    let mut tombstones: HashMap<String, String> = HashMap::new();
    let mut ineligible = Vec::new();
    for art in eligible {
        match mint(art, git_root) {
            Ok(id) => {
                tombstones.insert(art.id.clone(), policy.tombstone(art, &id));
            }
            Err(reason) => ineligible.push(Ineligible {
                artifact: art.clone(),
                reason,
            }),
        }
    }
    if !ineligible.is_empty() {
        return Err(Error::Ineligible(ineligible));
    }
    if eligible.is_empty() {
        return Ok(Plan::default());
    }

    let mut rewrites = Vec::new();
    for art in survivors {
        if let Some(rewrite) = plan_rewrite(art, policy, &tombstones)? {
            rewrites.push(rewrite);
        }
    }
    rewrites.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(Plan {
        rewrites,
        ..delete_plan(eligible)
    })
}

/// The plan that deletes every `eligible` artifact and rewrites nothing: no
/// git, no tombstone, no dirty-tree guard. Use it when [`referrers`] finds no
/// survivor that names a retired artifact, so nothing needs a tombstone.
#[must_use]
pub fn delete_plan(eligible: &[Artifact]) -> Plan {
    let mut deletions: Vec<Deletion> = eligible
        .iter()
        .map(|art| Deletion {
            abs: art.abs.clone(),
            rel: art.rel.clone(),
        })
        .collect();
    deletions.sort_by(|a, b| a.rel.cmp(&b.rel));
    Plan {
        deletions,
        rewrites: Vec::new(),
    }
}

/// Which retired identities each survivor references, keyed by the survivor's
/// display path. A survivor that names no retired artifact is left out, so an
/// empty map means nothing needs a tombstone. Uses the same
/// [`Policy::scan`] and [`Policy::identity`] as [`plan`], and needs no git.
///
/// # Errors
/// [`Error::Read`] on the first survivor that can't be read.
pub fn referrers<P: Policy>(
    eligible: &[Artifact],
    survivors: &[Artifact],
    policy: &P,
) -> Result<BTreeMap<PathBuf, BTreeSet<String>>, Error> {
    let retired: BTreeSet<&str> = eligible.iter().map(|art| art.id.as_str()).collect();
    let mut found = BTreeMap::new();
    for art in survivors {
        let source = read(art)?;
        let ids: BTreeSet<String> = named_targets(&source, policy)
            .into_iter()
            .map(|(_, id)| id)
            .filter(|id| retired.contains(id.as_str()))
            .collect();
        if !ids.is_empty() {
            found.insert(art.rel.clone(), ids);
        }
    }
    Ok(found)
}

/// Mint the tombstone for one eligible artifact, or say why it can't carry one.
fn mint(art: &Artifact, git_root: &Path) -> Result<Swhid, Reason> {
    let rel = art
        .abs
        .strip_prefix(git_root)
        .map_err(|_| Reason::OutsideRepo)?;
    swhid_mint::git::mint_rev_path(git_root, &slash_path(rel)).map_err(Reason::Mint)
}

/// A repository-relative path as git spells it, `/`-separated on every
/// platform, so the SWHID minted from it resolves on any machine.
fn slash_path(rel: &Path) -> String {
    rel.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// A survivor's source.
fn read(art: &Artifact) -> Result<String, Error> {
    std::fs::read_to_string(&art.abs).map_err(|err| Error::Read(art.rel.clone(), err))
}

/// Every reference the policy scans for in `source` that names an artifact,
/// with the identity it names.
fn named_targets<P: Policy>(source: &str, policy: &P) -> Vec<(Target, String)> {
    policy
        .scan(source)
        .into_iter()
        .filter_map(|target| {
            let id = policy.identity(&target.raw)?;
            Some((target, id))
        })
        .collect()
}

/// Build the rewrite for one survivor, or `None` if it references no retired
/// artifact. Reads the source, locates every reference the policy scans for, and
/// retargets those it resolves to a retired identity — leaving every other
/// reference untouched.
fn plan_rewrite<P: Policy>(
    art: &Artifact,
    policy: &P,
    tombstones: &HashMap<String, String>,
) -> Result<Option<Rewrite>, Error> {
    let source = read(art)?;
    let edits: Vec<typst_edit::Edit> = named_targets(&source, policy)
        .into_iter()
        .filter_map(|(target, id)| {
            let replacement = tombstones.get(&id)?;
            Some(typst_edit::Edit::new(target.range, replacement.clone()))
        })
        .collect();
    if edits.is_empty() {
        return Ok(None);
    }
    let links = edits.len();
    let contents =
        typst_edit::apply(&source, edits).map_err(|err| Error::Rewrite(art.rel.clone(), err))?;
    Ok(Some(Rewrite {
        abs: art.abs.clone(),
        rel: art.rel.clone(),
        contents,
        links,
    }))
}

/// Write every rewrite, then delete every retired file. Call only on a complete,
/// validated [`Plan`] from [`plan`]; does not commit.
///
/// # Errors
/// [`Error::Write`] on the first file write or deletion that fails.
pub fn apply(plan: &Plan) -> Result<(), Error> {
    for rewrite in &plan.rewrites {
        std::fs::write(&rewrite.abs, &rewrite.contents)
            .map_err(|err| Error::Write(rewrite.rel.clone(), err))?;
    }
    for deletion in &plan.deletions {
        std::fs::remove_file(&deletion.abs)
            .map_err(|err| Error::Write(deletion.rel.clone(), err))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
