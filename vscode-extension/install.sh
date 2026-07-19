#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

extension_name=$(node -p "require('./package.json').name")
extension_version=$(node -p "require('./package.json').version")
vsix_file="${extension_name}-${extension_version}.vsix"
editor_bin="${CODE_BIN:-}"

if [[ -z "${editor_bin}" ]]; then
  if command -v code >/dev/null 2>&1; then
    editor_bin="code"
  elif command -v cursor >/dev/null 2>&1; then
    editor_bin="cursor"
  elif command -v codium >/dev/null 2>&1; then
    editor_bin="codium"
  fi
fi

echo "Packaging ${extension_name} ${extension_version}..."
npx --yes @vscode/vsce package --allow-missing-repository --out "${vsix_file}"

if [[ -z "${editor_bin}" ]]; then
  cat <<EOF

Packaged ${vsix_file}.

No VS Code-compatible CLI was found on PATH.
Install manually from VS Code:
  Extensions -> ... -> Install from VSIX... -> $(pwd)/${vsix_file}

Or install from a terminal after enabling the VS Code shell command:
  code --install-extension "$(pwd)/${vsix_file}" --force
EOF
  exit 0
fi

echo "Installing ${vsix_file} with ${editor_bin}..."
"${editor_bin}" --install-extension "$(pwd)/${vsix_file}" --force
echo "Installed. Restart the editor if open .lum files do not refresh immediately."
