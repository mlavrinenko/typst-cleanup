# typst-cleanup

[![CI](https://github.com/mlavrinenko/typst-cleanup/actions/workflows/ci.yml/badge.svg)](https://github.com/mlavrinenko/typst-cleanup/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/typst-cleanup.svg)](https://crates.io/crates/typst-cleanup)
[![License: MIT](https://img.shields.io/crates/l/typst-cleanup.svg)](LICENSE-MIT)

Consumer-blind orchestration for retiring Typst artifacts: dirty-tree guard, SWHID tombstone minting, and atomic inbound-link rewriting

## Install

```bash
cargo add typst-cleanup
```

## Usage

```rust
use typst_cleanup::{Artifact, Policy, Swhid, apply, is_clean, plan, toplevel};

// The crate is consumer-blind: it knows nothing about what an artifact *is*.
// Supply identity (which link() URLs name artifacts) and tombstone policy.
struct Stems;

impl Policy for Stems {
    fn link_identity(&self, url: &str) -> Option<String> {
        Some(std::path::Path::new(url).file_stem()?.to_string_lossy().into_owned())
    }

    fn tombstone(&self, _retired: &Artifact, swhid: &Swhid) -> String {
        format!("\"{}\"", swhid.render())
    }
}

let root = toplevel(std::path::Path::new("."))?;
if !is_clean(&root)? {
    return Err("working tree is dirty".into());
}

// `plan` mints a tombstone for every eligible artifact and splices survivors'
// inbound links onto it, all in memory. `apply` writes and deletes; call it
// only once you're happy with the plan.
let computed = plan(&eligible, &survivors, &root, &Stems)?;
apply(&computed)?;
```

The crate does not commit — the caller reviews the diff and commits.

## Development

Prerequisites: [Nix](https://nixos.org/) with flakes enabled.

```bash
direnv allow         # or: nix develop

just check           # fmt + clippy + tests + file-size + drift check
just build
just test
just cover           # code coverage (70% minimum)
just fmt             # format code
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for coding conventions.

## License

MIT
