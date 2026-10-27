//! Why a cleanup plan or apply failed, and why an eligible artifact can't
//! carry a tombstone.

use std::path::PathBuf;

use crate::Artifact;

/// Why a cleanup plan or apply failed.
#[derive(Debug)]
pub enum Error {
    /// Eligible artifacts that can't carry a tombstone, every one of them, in
    /// the order they were given, so the caller can report them in one pass.
    Ineligible(Vec<Ineligible>),
    /// Reading a surviving file failed.
    Read(PathBuf, std::io::Error),
    /// Writing or deleting a file during apply failed.
    Write(PathBuf, std::io::Error),
    /// Splicing a surviving file's links failed (overlap, bad span).
    Rewrite(PathBuf, typst_edit::EditError),
    /// A repository or status query failed.
    Git(String),
}

/// An eligible artifact that can't carry a tombstone, and why.
#[derive(Debug)]
pub struct Ineligible {
    /// The artifact as it was passed to [`crate::plan`].
    pub artifact: Artifact,
    /// Why no tombstone can be minted for it.
    pub reason: Reason,
}

/// Why an eligible artifact can't carry a tombstone.
#[derive(Debug)]
pub enum Reason {
    /// The file lies outside the git repository root.
    OutsideRepo,
    /// Minting failed: the file is untracked, has uncommitted changes, or git
    /// failed.
    Mint(swhid_mint::git::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ineligible(found) => {
                write!(f, "can't mint a tombstone for")?;
                for (index, bad) in found.iter().enumerate() {
                    let sep = if index == 0 { " " } else { "; " };
                    write!(f, "{sep}{bad}")?;
                }
                Ok(())
            }
            Self::Read(path, err) => write!(f, "read {}: {err}", path.display()),
            Self::Write(path, err) => write!(f, "write {}: {err}", path.display()),
            Self::Rewrite(path, err) => write!(f, "rewrite links in {}: {err}", path.display()),
            Self::Git(msg) => write!(f, "git: {msg}"),
        }
    }
}

impl std::fmt::Display for Ineligible {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let path = self.artifact.rel.display();
        match &self.reason {
            Reason::OutsideRepo => write!(f, "{path}: outside the git repository"),
            Reason::Mint(err) => write!(f, "{path}: {err}"),
        }
    }
}

impl std::error::Error for Error {}
