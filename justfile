set positional-arguments

# Lists the available recipes.
[default]
help:
    @just --list

# Runs Shipdeck; pass a deploy root, a subcommand, or --help.
run *args:
    cargo run --locked -p shipdeck -- "$@"

# Runs Shipdeck with release optimizations.
run-release *args:
    cargo run --locked --release -p shipdeck -- "$@"

# Builds every workspace package; use `just build release` for optimized binaries.
build profile='dev':
    cargo build --workspace --locked --profile "$1"

# Builds optimized workspace binaries in target/release.
build-release: (build 'release')

alias release := build-release

# Installs the local release build into Cargo's bin folder (usually ~/.cargo/bin).
install:
    cargo install --path apps/cli --locked

# Installs the local release agent into Cargo's bin folder (usually ~/.cargo/bin).
install-agent:
    cargo install --path crates/agent --locked

# Checks every workspace target without producing binaries.
check:
    cargo check --workspace --all-targets --locked

# Runs workspace tests; extra arguments go to Cargo.
test *args:
    cargo test --workspace --locked "$@"

# Formats all Rust code.
fmt:
    cargo fmt --all

# Checks Rust formatting without changing files.
fmt-check:
    cargo fmt --all -- --check

# Runs the same strict Clippy checks as the pre-push hook.
lint:
    cargo clippy --workspace --all-targets --locked -- -D warnings -W clippy::pedantic -W clippy::perf

# Runs formatting, compilation, lint, and test checks in order.
verify: fmt-check check lint test

# Builds workspace API documentation without dependency docs.
doc:
    cargo doc --workspace --no-deps --locked

# Removes Cargo build artifacts.
clean:
    cargo clean

# Updates the workspace version and release links with a standalone Rust tool.
version new:
    #!/bin/sh
    set -eu
    build_dir=$(mktemp -d)
    trap 'rm -r "$build_dir"' EXIT
    rustc --edition=2024 scripts/bump-version.rs -o "$build_dir/bump-version"
    "$build_dir/bump-version" "$1"

# Writes the JSON Schema for deploy.yaml, and the website's copy of it.
schema:
    cargo run -q -p shipdeck -- --schema > docs/schema/deploy.schema.json
    cp docs/schema/deploy.schema.json docs/website/public/deploy.schema.json

# Writes the JSON Schema of the protocol messages, from their Rust types.
protocol:
    cargo run -q -p shipdeck-protocol --example schema > docs/schema/protocol.schema.json

# Builds and deploys the documentation website after refreshing its schema.
deploy-doc: schema
    cd ./docs/website/ && pnpm run deploy

# Starts the documentation website's development server.
dev-doc:
    cd ./docs/website && pnpm run dev

# Builds the documentation website after refreshing its schema.
build-doc: schema
    cd ./docs/website && pnpm run build
