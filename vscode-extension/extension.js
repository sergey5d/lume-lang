const path = require("node:path");
const vscode = require("vscode");

const { formatText } = require("./formatter");
const { findDefinition } = require("./navigation");

function configuredExecutable(configuration) {
  const configured = configuration.get("executable.path", "").trim();
  if (configured) {
    return configured;
  }
  return configuration.get("formatter.path", "lume").trim() || "lume";
}

function workingDirectory(document) {
  const workspace = vscode.workspace.getWorkspaceFolder(document.uri);
  if (workspace) {
    return workspace.uri.fsPath;
  }
  return document.uri.scheme === "file"
    ? path.dirname(document.uri.fsPath)
    : undefined;
}

function activate(context) {
  const output = vscode.window.createOutputChannel("Lume");
  const provider = {
    async provideDocumentFormattingEdits(document, _options, token) {
      const configuration = vscode.workspace.getConfiguration("lume", document.uri);
      const executable = configuredExecutable(configuration);
      const cwd = workingDirectory(document);
      const controller = new AbortController();
      if (token.isCancellationRequested) {
        controller.abort();
      }
      const cancellation = token.onCancellationRequested(() => controller.abort());

      try {
        const source = document.getText();
        const formatted = await formatText(source, {
          executable,
          cwd,
          signal: controller.signal,
        });
        if (formatted === source) {
          return [];
        }

        const wholeDocument = new vscode.Range(
          new vscode.Position(0, 0),
          document.positionAt(source.length),
        );
        return [vscode.TextEdit.replace(wholeDocument, formatted)];
      } catch (error) {
        if (token.isCancellationRequested) {
          return [];
        }
        const message = error instanceof Error ? error.message : String(error);
        throw new Error(`Lume formatting failed: ${message}`);
      } finally {
        cancellation.dispose();
      }
    },
  };

  const definitionProvider = {
    async provideDefinition(document, position, token) {
      if (document.uri.scheme !== "file") {
        return undefined;
      }
      const configuration = vscode.workspace.getConfiguration("lume", document.uri);
      const source = document.getText();
      const utf16Offset = document.offsetAt(position);
      const offset = Buffer.byteLength(source.slice(0, utf16Offset), "utf8");
      const controller = new AbortController();
      if (token.isCancellationRequested) {
        controller.abort();
      }
      const cancellation = token.onCancellationRequested(() => controller.abort());

      try {
        const result = await findDefinition(source, {
          executable: configuredExecutable(configuration),
          sourcePath: document.uri.fsPath,
          offset,
          cwd: workingDirectory(document),
          signal: controller.signal,
        });
        if (!result || result.path.startsWith("<")) {
          return undefined;
        }
        const target = new vscode.Position(
          Math.max(0, result.line - 1),
          Math.max(0, result.column - 1),
        );
        return new vscode.Location(vscode.Uri.file(result.path), target);
      } catch (error) {
        if (!token.isCancellationRequested) {
          const message = error instanceof Error ? error.message : String(error);
          output.appendLine(`Definition lookup failed: ${message}`);
        }
        return undefined;
      } finally {
        cancellation.dispose();
      }
    },
  };

  context.subscriptions.push(
    vscode.languages.registerDocumentFormattingEditProvider("lume", provider),
    vscode.languages.registerDefinitionProvider("lume", definitionProvider),
    vscode.commands.registerCommand("lume.formatDocument", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor || editor.document.languageId !== "lume") {
        await vscode.window.showErrorMessage("Open a Lume file to format it.");
        return;
      }
      await vscode.commands.executeCommand("editor.action.formatDocument");
    }),
    output,
  );
}

function deactivate() {}

module.exports = { activate, deactivate };
