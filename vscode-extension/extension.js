const path = require("node:path");
const vscode = require("vscode");

const { formatText } = require("./formatter");

function activate(context) {
  const provider = {
    async provideDocumentFormattingEdits(document, _options, token) {
      const configuration = vscode.workspace.getConfiguration("lume", document.uri);
      const configuredPath = configuration.get("formatter.path", "lume");
      const executable = configuredPath.trim() || "lume";
      const workspace = vscode.workspace.getWorkspaceFolder(document.uri);
      const cwd = workspace
        ? workspace.uri.fsPath
        : document.uri.scheme === "file"
          ? path.dirname(document.uri.fsPath)
          : undefined;
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

  context.subscriptions.push(
    vscode.languages.registerDocumentFormattingEditProvider("lume", provider),
    vscode.commands.registerCommand("lume.formatDocument", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor || editor.document.languageId !== "lume") {
        await vscode.window.showErrorMessage("Open a Lume file to format it.");
        return;
      }
      await vscode.commands.executeCommand("editor.action.formatDocument");
    }),
  );
}

function deactivate() {}

module.exports = { activate, deactivate };
