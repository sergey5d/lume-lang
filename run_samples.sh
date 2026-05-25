#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODE="${1:-rust}"

run_go_samples() {
  (
    cd "$ROOT"
    GOCACHE="$ROOT/.gocache" go test -v examples/examples_test.go
  )
}

run_rust_samples() {
  (
    cd "$ROOT"
    cargo test --manifest-path rust/Cargo.toml -p lume \
      interpreter::tests::run_path_matches_expected_output_headers_for_examples \
      -- --exact
  )
}

case "$MODE" in
  go)
    run_go_samples
    ;;
  rust)
    run_rust_samples
    ;;
  both)
    run_go_samples
    run_rust_samples
    ;;
  *)
    echo "usage: ./run_samples.sh [go|rust|both]" >&2
    exit 2
    ;;
esac
