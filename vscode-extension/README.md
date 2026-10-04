# Lume VS Code Extension

This VS Code extension adds:

- `.lum` file association
- line comments with `#`
- bracket / auto-close rules
- TextMate-based syntax highlighting for Lume
- document formatting through `lume fmt`

## Format Lume source

Install `lume` on your `PATH`, open a `.lum` file, and run **Format Document**
or **Lume: Format Document** from the Command Palette. The formatter operates
on the current editor contents, including unsaved changes, and applies the
result as an editor change that can be undone normally.

To use a compiler at another location, configure its path:

```json
{
  "lume.formatter.path": "/absolute/path/to/lume"
}
```

Formatting on save uses the standard VS Code settings:

```json
{
  "[lume]": {
    "editor.defaultFormatter": "sergeyd.lume-syntax",
    "editor.formatOnSave": true
  }
}
```

## Install locally

From this repo:

```sh
cd vscode-extension
./install.sh
```

The script packages the extension as `lume-syntax-0.0.2.vsix` and installs it
with the first VS Code-compatible CLI it finds: `code`, `cursor`, or `codium`.
It uses `npx @vscode/vsce`, so Node/npm must be available.

If your editor CLI is not on `PATH`, pass it explicitly:

```sh
CODE_BIN=/path/to/code ./install.sh
```

For regular VS Code on macOS, install the `code` command first:

1. Open VS Code.
2. Run `Shell Command: Install 'code' command in PATH` from the Command Palette.
3. Run `./install.sh` again.

Manual install also works:

```sh
cd vscode-extension
npx --yes @vscode/vsce package --allow-missing-repository --out lume-syntax-0.0.2.vsix
code --install-extension "$(pwd)/lume-syntax-0.0.2.vsix" --force
```

Or use the UI:

1. Open VS Code.
2. Open the Extensions view.
3. Click the `...` menu.
4. Choose `Install from VSIX...`.
5. Select `vscode-extension/lume-syntax-0.0.1.vsix`.

For development, the easiest path is:

1. Open `vscode-extension` in VS Code.
2. Press `F5`.
3. A new Extension Development Host window opens.
4. Open any `.lum` file there to see highlighting.

## Notes

This extension remains intentionally lightweight:

- no language server
- no semantic analysis
