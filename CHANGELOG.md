# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-07-05

### Added

- Initial release, extracted from the `mindtape` workspace. Consumer-blind
  retire-artifact orchestration: `plan`/`apply` with a dirty-tree guard
  ([`is_clean`], [`toplevel`]), SWHID tombstone minting via `swhid-mint`, and
  atomic inbound-link rewriting via `typst-edit`.
