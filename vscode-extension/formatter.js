const childProcess = require("node:child_process");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

async function formatText(source, options = {}) {
  const executable = options.executable || "lume";
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), "lume-vscode-fmt-"));
  const sourcePath = path.join(directory, "document.lum");

  try {
    await fs.writeFile(sourcePath, source, "utf8");
    await runFormatter(executable, sourcePath, options.cwd, options.signal);
    return await fs.readFile(sourcePath, "utf8");
  } finally {
    await fs.rm(directory, { recursive: true, force: true });
  }
}

function runFormatter(executable, sourcePath, cwd, signal) {
  return new Promise((resolve, reject) => {
    let stderr = "";
    const child = childProcess.spawn(executable, ["fmt", sourcePath], {
      cwd,
      signal,
      windowsHide: true,
    });

    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", (error) => {
      if (error.name === "AbortError") {
        reject(error);
      } else if (error.code === "ENOENT") {
        reject(
          new Error(
            `cannot find '${executable}'; install Lume or set lume.formatter.path`,
          ),
        );
      } else {
        reject(error);
      }
    });
    child.on("close", (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(
          new Error(
            stderr.trim() || `lume fmt exited with status ${String(code)}`,
          ),
        );
      }
    });
  });
}

module.exports = { formatText };
