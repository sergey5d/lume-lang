#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MODE="${1:-rust}"

run_rust_samples() {
  (
    cd "$ROOT"
    cargo test --manifest-path rust/Cargo.toml -p lume \
      interpreter::tests::run_path_matches_all_headers_for_examples \
      -- --exact --nocapture
  )
}

case "$MODE" in
  rust)
    run_rust_samples
    ;;
  *)
    echo "usage: ./run_samples.sh [rust]" >&2
    exit 2
    ;;
esac
