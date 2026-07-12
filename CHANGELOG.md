# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-07-12

### Added

- `Policy::scan` locates the rewritable references in a survivor's source,
  defaulting to `link("…")` calls. Override it to retarget a DSL's own edge
  helpers (`depends-on("x.typ")`, `edge(kind, …)`) — the crate keeps owning the
  mint, the dirty-tree guard, and the atomic `plan`/`apply`. Fixes the case
  where inbound edges are authored as helper calls over plain strings, never
  `link(...)`, which the old link-only scan could not see.
- `Target { raw, range }`: a located rewritable reference, returned by `scan`
  and fed straight to `typst_edit::Edit`.

### Changed

- `Policy::link_identity` renamed to `Policy::identity`: it now maps any raw
  target string (from `scan`), not only a `link()` URL. Breaking — rename the
  method in your `Policy` impl.

## [0.1.0] - 2026-07-05

### Added

- Initial release, extracted from the `mindtape` workspace. Consumer-blind
  retire-artifact orchestration: `plan`/`apply` with a dirty-tree guard
  ([`is_clean`], [`toplevel`]), SWHID tombstone minting via `swhid-mint`, and
  atomic inbound-link rewriting via `typst-edit`.
