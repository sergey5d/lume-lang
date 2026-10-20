const childProcess = require("node:child_process");

function findDefinition(source, options) {
  const executable = options.executable || "lume";
  const sourcePath = options.sourcePath;
  const offset = options.offset;

  return new Promise((resolve, reject) => {
    let stdout = "";
    let stderr = "";
    const child = childProcess.spawn(
      executable,
      ["definition", sourcePath, "--offset", String(offset), "--stdin"],
      {
        cwd: options.cwd,
        signal: options.signal,
        windowsHide: true,
      },
    );

    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", (error) => {
      if (error.name === "AbortError") {
        reject(error);
      } else if (error.code === "ENOENT") {
        reject(
          new Error(
            `cannot find '${executable}'; install Lume or set lume.executable.path`,
          ),
        );
      } else {
        reject(error);
      }
    });
    child.on("close", (code) => {
      if (code !== 0) {
        reject(
          new Error(
            stderr.trim() ||
              `lume definition exited with status ${String(code)}`,
          ),
        );
        return;
      }

      try {
        const result = JSON.parse(stdout);
        if (result === null) {
          resolve(undefined);
          return;
        }
        if (
          typeof result.path !== "string" ||
          typeof result.line !== "number" ||
          typeof result.column !== "number"
        ) {
          throw new Error("lume definition returned an invalid location");
        }
        resolve(result);
      } catch (error) {
        reject(error);
      }
    });

    child.stdin.on("error", (error) => {
      if (error.code !== "EPIPE") {
        reject(error);
      }
    });
    child.stdin.end(source, "utf8");
  });
}

module.exports = { findDefinition };
