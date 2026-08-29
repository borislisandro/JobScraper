import { execFileSync, spawn } from "node:child_process";
import { createServer } from "node:http";
import { access } from "node:fs/promises";
import { join } from "node:path";

const installRoot = process.argv[2];
if (!installRoot) throw new Error("Pass the installed JobScraper directory.");
const node = join(installRoot, "sidecar", "node.exe");
const worker = join(installRoot, "sidecar", "worker.mjs");
await Promise.all([access(node), access(worker)]);

function edgePids() {
  const output = execFileSync(
    "tasklist.exe",
    ["/FI", "IMAGENAME eq msedge.exe", "/FO", "CSV", "/NH"],
    { encoding: "utf8" },
  );
  return new Set(
    [...output.matchAll(/^"msedge\.exe","(\d+)"/gim)].map((match) => match[1]),
  );
}

const before = edgePids();
const server = createServer((_request, response) => {
  response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
  response.end(
    "<!doctype html><div class='job' data-job-id='one'><h2>Firmware Engineer</h2><a href='/apply'>Apply</a><p>Embedded systems role</p></div>",
  );
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));

try {
  const address = server.address();
  if (!address || typeof address === "string") throw new Error("Fixture server failed.");
  const child = spawn(node, [worker], { stdio: ["pipe", "pipe", "pipe"] });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8").on("data", (chunk) => (stdout += chunk));
  child.stderr.setEncoding("utf8").on("data", (chunk) => (stderr += chunk));
  child.stdin.end(
    `${JSON.stringify({
      protocolVersion: 1,
      command: "test_source",
      runId: "installed-edge-smoke",
      source: {
        id: "installed-edge-smoke",
        name: "Local fixture",
        baseUrl: `http://127.0.0.1:${address.port}/jobs`,
        adapterId: "playwright",
        adapterVersion: "1.1.0",
        kind: "active",
        headless: true,
        allowPrivateNetwork: true,
        configJson: { mode: "playwright", testNoDelay: true },
      },
    })}\n`,
  );
  let timeout;
  const exitCode = await Promise.race([
    new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", resolve);
    }),
    new Promise((_, reject) => {
      timeout = setTimeout(() => {
        child.kill();
        reject(new Error("Installed worker smoke timed out."));
      }, 45_000);
    }),
  ]).finally(() => clearTimeout(timeout));
  if (exitCode !== 0) throw new Error(`Worker exited ${exitCode}: ${stderr}`);
  const events = stdout
    .trim()
    .split(/\r?\n/)
    .filter(Boolean)
    .map((line) => JSON.parse(line));
  if (events[0]?.event !== "started") throw new Error("Missing started event.");
  if (!events.some((event) => event.event === "job" && event.payload?.title === "Firmware Engineer")) {
    throw new Error(`Installed Edge did not extract fixture job: ${stdout}`);
  }
  if (events.at(-1)?.event !== "completed" || events.at(-1)?.payload?.complete !== true) {
    throw new Error(`Missing complete traversal event: ${stdout}`);
  }
  await new Promise((resolve) => setTimeout(resolve, 2_000));
  const leaked = [...edgePids()].filter((pid) => !before.has(pid));
  if (leaked.length) throw new Error(`Edge process leak after worker exit: ${leaked.join(", ")}`);
  process.stdout.write(`${JSON.stringify({ events: events.map((event) => event.event), leakedEdgePids: leaked })}\n`);
} finally {
  await new Promise((resolve) => server.close(resolve));
}
