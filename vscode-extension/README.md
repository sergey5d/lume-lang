# Lume VS Code Extension

This is a minimal VS Code extension that adds:

- `.lum` file association
- line comments with `#`
- bracket / auto-close rules
- TextMate-based syntax highlighting for Lume

## Install locally

1. Open VS Code.
2. Open the Extensions view.
3. Click the `...` menu in the top-right.
4. Choose `Install from VSIX...` if you package it, or use `Developer: Install Extension from Location...` if your VS Code build exposes that action.

For development, the easiest path is:

1. Open `vscode-extension` in VS Code.
2. Press `F5`.
3. A new Extension Development Host window opens.
4. Open any `.lum` file there to see highlighting.

## Notes

This extension is intentionally lightweight:

- no language server
- no formatter
- no semantic analysis

It is just syntax support for now.
