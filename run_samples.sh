#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ $# -ne 0 ]]; then
  echo "usage: ./run_samples.sh" >&2
  exit 2
fi

cd "$ROOT"
cargo test --manifest-path rust/Cargo.toml -p lume \
  interpreter::tests::run_path_matches_all_headers_for_examples \
  -- --exact --nocapture
