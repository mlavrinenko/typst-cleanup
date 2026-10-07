set quiet := true

# List available recipes
default:
    @just --list

# Run fixes, then other checks
fix-check: eject fmt clippy-fix check

# Run all checks in parallel (fmt + clippy + tests + unused deps + file size + drift)
check:
    parallel -j 0 -- \
        "chronic just fmt-check" \
        "chronic cargo clippy --workspace --all-targets -q -- -D warnings" \
        "chronic cargo test --workspace -q" \
        "chronic just machete" \
        "chronic just check-file-size" \
        "chronic just outdatty-check"

# Fail if a source changed without its dependents being re-confirmed
outdatty-check:
    outdatty check --format quiet

# Re-confirm dependency groups: record current hashes into outdatty.lock
outdatty-update:
    outdatty update

# Run tests only
test *ARGS:
    cargo test --workspace {{ ARGS }}

# Run clippy only
clippy:
    cargo clippy --workspace --all-targets -q -- -D warnings

# Auto-fix clippy warnings (allow-dirty/-staged: fix-check runs pre-commit, tree is dirty).
# Restore write on target/ first: tarpaulin (`just cover`/`just crap`) leaves *.rmeta
# artifacts read-only, and `clippy --fix` then aborts with "output file ... is not
# writeable". Cheap self-heal so `fix-check` after `cover`/`crap` never trips on it.
clippy-fix:
    chmod -R u+w target 2>/dev/null || true
    cargo clippy --fix --workspace --all-targets --allow-dirty --allow-staged -- -D warnings

# Build the project
build *args:
    cargo build --workspace -q {{ args }}

# Run coverage with tarpaulin (also writes target/coverage/lcov.info).
# --all-features so optional-feature code is measured, not silently skipped.
cover:
    cargo tarpaulin --workspace --all-features --skip-clean

# Gate complex, undertested functions via CRAP metric. Needs lcov from `just
# cover` first. Threshold 30 is a sane greenfield default; tune per repo.
# cargo-crap has no --all-features flag: it statically reads source + the lcov
# already produced by `just cover --all-features`, it doesn't build anything.
crap:
    cargo crap --lcov target/coverage/lcov.info --workspace --exclude 'src/main.rs' --threshold 30 --fail-above

# Format code
fmt:
    cargo fmt --all

# Format check (CI-friendly)
fmt-check:
    cargo fmt --all -- --check

# Check for unused dependencies
machete:
    cargo machete

# Count tests across workspace
count-tests:
    #!/usr/bin/env bash
    cargo test --workspace 2>&1 | grep "test result:" | awk '{sum += $4} END {print sum " tests"}'

# Show top 20 files by line count
file-sizes:
    #!/usr/bin/env bash
    find . -type f \( -name '*.rs' -o -name '*.md' \) ! -path './target/*' -exec wc -l {} + | sort -rn | head -20

# Eject inline tests from Rust files nearing the linecop limit, so they stay
# under it without losing the inline-test workflow. Runs as part of `fix-check`.
eject PCT='90':
    linecop --baseline {{ PCT }} --format paths | ejectest apply src --files-from - --lenient

# Check for oversized files (fails if any exceed limits)
check-file-size:
    linecop

# The tag push is the only publish path: never `cargo publish` by hand, since a
# version already on crates.io used to fail the tag's workflow and leave the
# release half-done. Each refusal guards one way a release has gone wrong and
# prints `error:` plus a `hint:` with the way out. `--dry-run` runs every check
# and stops before tagging. A tag whose workflow failed is re-run on the same
# tag, never re-tagged: `gh workflow run <release workflow> -f tag=vX.Y.Z`.
# Tag a release and push the tag (usage: just release 0.1.0 [--dry-run])
[no-exit-message]
release VERSION *FLAGS:
    #!/usr/bin/env bash
    set -euo pipefail
    version='{{ VERSION }}'
    tag="v$version"
    dry_run=false
    refuse() { echo "error: $1" >&2; echo "hint: $2" >&2; exit 1; }
    for flag in {{ FLAGS }}; do
        case "$flag" in
            --dry-run) dry_run=true ;;
            *) refuse "unknown flag $flag" "just release VERSION [--dry-run]" ;;
        esac
    done
    [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] \
        || refuse "'$version' is not a semver version" "give it without the v: just release 1.2.3"
    cargo_version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
    [ "$version" = "$cargo_version" ] \
        || refuse "requested $tag but Cargo.toml is $cargo_version" "bump Cargo.toml to $version and commit it first"
    for tool in git cargo curl gh; do
        command -v "$tool" >/dev/null \
            || refuse "$tool is not on PATH" "run inside the dev shell, with gh authenticated (gh auth login)"
    done
    crate=$(sed -n 's/^name = "\(.*\)"/\1/p' Cargo.toml | head -1)
    workflow=$(basename "$(ls .github/workflows/release*.yml | head -1)")
    [ -z "$(git status --porcelain)" ] \
        || refuse "the working tree has uncommitted changes" "commit or discard them, then re-run"
    branch=$(git symbolic-ref --short -q HEAD || true)
    [ "$branch" = main ] || refuse "HEAD is on '${branch:-a detached commit}', not main" "git switch main"
    git fetch --quiet origin main || refuse "cannot fetch origin main" "check the network and the origin remote"
    [ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] \
        || refuse "main and origin/main differ, so the tag would name a commit CI never saw" "push or pull until main matches origin/main"
    if git rev-parse -q --verify "refs/tags/$tag" >/dev/null \
        || git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null; then
        refuse "$tag already exists" "a pushed tag never moves; finish a failed release with: gh workflow run $workflow -f tag=$tag"
    fi
    status=$(curl -sS -o /dev/null -w '%{http_code}' -A "$crate release preflight" \
        "https://crates.io/api/v1/crates/$crate/$version" || true)
    case "$status" in
        404) ;;
        200) refuse "$crate $version is already on crates.io" "a published version is never re-published; bump to the next version" ;;
        *) refuse "crates.io answered HTTP ${status:-nothing} for $crate $version" "retry once crates.io is reachable" ;;
    esac
    grep -Eq "^## \[${version//./\\.}\] - [0-9]{4}-[0-9]{2}-[0-9]{2}" CHANGELOG.md \
        || refuse "CHANGELOG.md has no dated section for $version" "move [Unreleased] under '## [$version] - $(date +%F)' and commit"
    ci=$(gh run list --commit "$(git rev-parse HEAD)" --workflow ci.yml --limit 1 \
        --json status,conclusion --jq '.[] | "\(.status) \(.conclusion)"') \
        || refuse "cannot list CI runs for HEAD" "check gh auth status and that .github/workflows/ci.yml exists"
    case "$ci" in
        "completed success") ;;
        "") refuse "CI has not run on HEAD" "push main and wait for ci.yml to pass" ;;
        completed*) refuse "CI did not pass on HEAD (${ci#completed })" "fix main until ci.yml is green: gh run list --workflow ci.yml" ;;
        *) refuse "CI is still running on HEAD" "wait for it: gh run watch" ;;
    esac
    just check || refuse "just check failed on this tree" "fix what it reports above, commit, and push"
    cargo publish --dry-run --locked --quiet || refuse "cargo publish --dry-run failed" "fix the packaging error above"
    if $dry_run; then
        echo "dry run: every check passed; $tag is ready to tag and push"
        exit 0
    fi
    git tag -a "$tag" -m "$tag"
    git push origin "$tag"
    echo "pushed $tag; follow it with: gh run list --workflow $workflow --limit 1"
