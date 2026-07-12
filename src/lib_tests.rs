#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use swhid_mint::Swhid;
use swhid_mint::test_support::repo_with;

use super::{Artifact, Error, Plan, Policy, Target, apply, is_clean, plan, toplevel};

/// A test policy: stems name artifacts; `swh:` URLs are existing tombstones.
struct Stems;

impl Policy for Stems {
    fn identity(&self, raw: &str) -> Option<String> {
        if raw.starts_with("swh:") {
            return None;
        }
        Some(Path::new(raw).file_stem()?.to_string_lossy().into_owned())
    }

    fn tombstone(&self, _retired: &Artifact, swhid: &Swhid) -> String {
        format!("\"{}\"", swhid.render())
    }
}

/// A policy whose edges are `depends-on("…")` helper calls, not `link(...)`. It
/// overrides `scan` to find them on the parse plane — the whole point of the
/// pluggable scan, and the shape a task tracker (e.g. mindtape) authors.
struct HelperEdges;

impl Policy for HelperEdges {
    fn scan(&self, source: &str) -> Vec<Target> {
        typst_edit::find_calls(source, "depends-on")
            .into_iter()
            .flat_map(|call| call.args)
            .filter(|arg| {
                arg.name.is_none() && source.as_bytes().get(arg.value_range.start) == Some(&b'"')
            })
            .map(|arg| Target {
                raw: arg.value,
                range: arg.value_range,
            })
            .collect()
    }

    fn identity(&self, raw: &str) -> Option<String> {
        Some(Path::new(raw).file_stem()?.to_string_lossy().into_owned())
    }

    fn tombstone(&self, _retired: &Artifact, swhid: &Swhid) -> String {
        format!("\"{}\"", swhid.render())
    }
}

fn artifact(root: &Path, rel: &str) -> Artifact {
    let path = PathBuf::from(rel);
    let id = path.file_stem().unwrap().to_string_lossy().into_owned();
    Artifact {
        abs: root.join(rel),
        rel: path,
        id,
    }
}

const LIVE: &str = "#show: task.with(\n  depends-on: (link(\"gone.typ\"),),\n)\n";

#[test]
fn empty_plan_when_nothing_eligible() {
    let (_dir, root) = repo_with(&[("live.typ", LIVE)]);
    let p = plan(&[], &[artifact(&root, "live.typ")], &root, &Stems).unwrap();
    assert_eq!(p, Plan::default());
    assert!(p.is_empty());
}

#[test]
fn plan_mints_tombstone_and_rewrites_survivor() {
    let (_dir, root) = repo_with(&[("gone.typ", "gone\n"), ("live.typ", LIVE)]);
    let eligible = vec![artifact(&root, "gone.typ")];
    let survivors = vec![artifact(&root, "live.typ")];

    let p = plan(&eligible, &survivors, &root, &Stems).unwrap();

    assert_eq!(p.deletions.len(), 1);
    assert_eq!(p.deletions[0].rel, PathBuf::from("gone.typ"));
    assert_eq!(p.rewrites.len(), 1);
    assert_eq!(p.rewrites[0].links, 1);
    assert!(p.rewrites[0].contents.contains("swh:1:rev:"));
    assert!(p.rewrites[0].contents.contains("path=gone.typ"));
    assert!(!p.rewrites[0].contents.contains("link(\"gone.typ\")"));
}

#[test]
fn plan_writes_nothing_until_apply() {
    // The dry-run guarantee: plan() touches no disk; only apply() writes.
    let (_dir, root) = repo_with(&[("gone.typ", "gone\n"), ("live.typ", LIVE)]);
    let before = std::fs::read_to_string(root.join("live.typ")).unwrap();

    let p = plan(
        &[artifact(&root, "gone.typ")],
        &[artifact(&root, "live.typ")],
        &root,
        &Stems,
    )
    .unwrap();

    assert!(root.join("gone.typ").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("live.typ")).unwrap(),
        before
    );

    apply(&p).unwrap();
    assert!(!root.join("gone.typ").exists());
    assert!(
        std::fs::read_to_string(root.join("live.typ"))
            .unwrap()
            .contains("swh:1:rev:")
    );
}

#[test]
fn multiple_survivors_all_rewritten() {
    let (_dir, root) = repo_with(&[("gone.typ", "gone\n"), ("a.typ", LIVE), ("b.typ", LIVE)]);
    let p = plan(
        &[artifact(&root, "gone.typ")],
        &[artifact(&root, "a.typ"), artifact(&root, "b.typ")],
        &root,
        &Stems,
    )
    .unwrap();
    assert_eq!(p.rewrites.len(), 2);
    assert!(p.rewrites.iter().all(|r| r.contents.contains("swh:1:rev:")));
}

#[test]
fn aborts_all_or_nothing_when_one_file_untracked() {
    let (_dir, root) = repo_with(&[("gone.typ", "gone\n")]);
    std::fs::write(root.join("orphan.typ"), "orphan\n").unwrap();

    let result = plan(
        &[artifact(&root, "gone.typ"), artifact(&root, "orphan.typ")],
        &[],
        &root,
        &Stems,
    );

    assert!(result.is_err());
    assert!(root.join("gone.typ").exists());
    assert!(root.join("orphan.typ").exists());
}

#[test]
fn dirty_tree_refusal_guard() {
    let (_dir, root) = repo_with(&[("a.typ", "a\n")]);
    assert!(is_clean(&root).unwrap());
    std::fs::write(root.join("a.typ"), "changed\n").unwrap();
    assert!(!is_clean(&root).unwrap());
}

#[test]
fn dirty_tree_counts_untracked() {
    let (_dir, root) = repo_with(&[("a.typ", "a\n")]);
    std::fs::write(root.join("new.typ"), "new\n").unwrap();
    assert!(!is_clean(&root).unwrap());
}

#[test]
fn toplevel_from_subdir() {
    let (_dir, root) = repo_with(&[("a.typ", "a\n")]);
    let sub = root.join("nested");
    std::fs::create_dir_all(&sub).unwrap();
    assert_eq!(toplevel(&sub).unwrap().canonicalize().unwrap(), root);
}

#[test]
fn survivor_read_error_aborts_plan() {
    // The survivor is declared but never written to disk: plan() must surface
    // the read failure via Error::Read rather than panicking or silently
    // skipping it.
    let (_dir, root) = repo_with(&[("gone.typ", "gone\n")]);
    let result = plan(
        &[artifact(&root, "gone.typ")],
        &[artifact(&root, "missing.typ")],
        &root,
        &Stems,
    );
    assert!(matches!(result, Err(Error::Read(..))));
}

#[test]
fn existing_tombstone_links_left_untouched() {
    let survivor = "#show: task.with(\n  relates-to: (link(\"swh:1:rev:0000000000000000000000000000000000000000;path=x.typ\"),),\n)\n";
    let (_dir, root) = repo_with(&[("gone.typ", "gone\n"), ("live.typ", survivor)]);
    let p = plan(
        &[artifact(&root, "gone.typ")],
        &[artifact(&root, "live.typ")],
        &root,
        &Stems,
    )
    .unwrap();
    // The survivor's only link is an existing tombstone, not an edge to `gone`.
    assert!(p.rewrites.is_empty());
}

#[test]
fn scan_override_retargets_helper_edges() {
    // A survivor whose inbound edge is `depends-on("gone.typ")` — never
    // `link(...)`. The default scan would miss it; `HelperEdges::scan` finds it,
    // and the crate retargets it onto the tombstone just like a link edge.
    let live = "#show: task.with(\n  links: depends-on(\"gone.typ\"),\n)\n";
    let (_dir, root) = repo_with(&[("gone.typ", "gone\n"), ("live.typ", live)]);

    let p = plan(
        &[artifact(&root, "gone.typ")],
        &[artifact(&root, "live.typ")],
        &root,
        &HelperEdges,
    )
    .unwrap();

    assert_eq!(p.rewrites.len(), 1);
    assert_eq!(p.rewrites[0].links, 1);
    assert!(p.rewrites[0].contents.contains("swh:1:rev:"));
    assert!(!p.rewrites[0].contents.contains("\"gone.typ\""));
}
