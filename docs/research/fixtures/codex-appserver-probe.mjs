// Probe for `codex app-server` (stdio JSONL JSON-RPC, "jsonrpc" header omitted).
// Usage: node codex-appserver-probe.mjs <cwd> <outfile>
// Turn 1: run `ls` + create a file via the editing tool (approvalPolicy=untrusted -> approvals are auto-accepted here).
// Turn 2: long answer, interrupted with turn/interrupt shortly after it starts.
import { spawn } from "node:child_process";
import { createWriteStream } from "node:fs";
import readline from "node:readline";

const [cwd, outFile] = process.argv.slice(2);
const out = createWriteStream(outFile);
const t0 = Date.now();
const REDACT = new Set(["email", "accountId", "userId", "account_id", "user_id"]);
const redact = (v) =>
  JSON.parse(JSON.stringify(v, (k, x) => (REDACT.has(k) && typeof x === "string" ? "<redacted>" : x)));
const log = (dir, msg) => out.write(JSON.stringify({ dir, t_ms: Date.now() - t0, msg: redact(msg) }) + "\n");

const child = spawn("codex", ["app-server", "-c", "mcp_servers={}"], { cwd, stdio: ["pipe", "pipe", "pipe"] });
child.stderr.on("data", (d) => out.write(JSON.stringify({ dir: "stderr", t_ms: Date.now() - t0, text: String(d).slice(0, 2000) }) + "\n"));

let nextId = 0;
const pending = new Map();
const send = (method, params) => {
  const id = nextId++;
  const msg = { method, id, params };
  log("->", msg);
  child.stdin.write(JSON.stringify(msg) + "\n");
  return new Promise((res) => pending.set(id, res));
};
const notify = (method, params) => {
  const msg = params === undefined ? { method } : { method, params };
  log("->", msg);
  child.stdin.write(JSON.stringify(msg) + "\n");
};
const reply = (id, result) => {
  const msg = { id, result };
  log("->", msg);
  child.stdin.write(JSON.stringify(msg) + "\n");
};

const waiters = [];
const waitFor = (pred) => new Promise((res) => waiters.push({ pred, res }));

readline.createInterface({ input: child.stdout, crlfDelay: Infinity }).on("line", (line) => {
  let msg;
  try { msg = JSON.parse(line); } catch { out.write(JSON.stringify({ dir: "<-raw", line }) + "\n"); return; }
  log("<-", msg);
  if (msg.id !== undefined && msg.method) {
    // Server-initiated request (approvals etc.)
    if (msg.method === "item/commandExecution/requestApproval" || msg.method === "item/fileChange/requestApproval") {
      reply(msg.id, { decision: "accept" });
    } else if (msg.method === "item/permissions/requestApproval") {
      reply(msg.id, { permissions: {}, scope: "turn" });
    } else {
      const err = { id: msg.id, error: { code: -32601, message: "probe: unsupported" } };
      log("->", err);
      child.stdin.write(JSON.stringify(err) + "\n");
    }
  } else if (msg.id !== undefined && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  }
  for (let i = waiters.length - 1; i >= 0; i--) if (waiters[i].pred(msg)) { waiters[i].res(msg); waiters.splice(i, 1); }
});

const hardStop = setTimeout(() => { out.write(JSON.stringify({ dir: "probe", note: "hard timeout" }) + "\n"); child.kill("SIGTERM"); }, 110_000);

(async () => {
  const init = await send("initialize", {
    clientInfo: { name: "codly_probe", title: "Codly probe", version: "0.0.1" },
    capabilities: { experimentalApi: false },
  });
  notify("initialized");
  await send("account/read", { refreshToken: false });
  await send("account/rateLimits/read");
  await send("model/list", { limit: 3 });
  const ts = await send("thread/start", {
    model: "gpt-6-luna",
    cwd,
    approvalPolicy: "untrusted",
    sandbox: "workspace-write",
    config: { model_reasoning_effort: "low" },
  });
  const threadId = ts.result?.thread?.id;
  if (!threadId) throw new Error("no thread id: " + JSON.stringify(ts));

  // Turn 1
  await send("turn/start", {
    threadId,
    input: [{ type: "text", text: "First run the shell command `ls`. Then create a new file named notes.txt containing the single line hello using your file editing tool (apply_patch), not the shell. Then reply with exactly: DONE", text_elements: [] }],
    effort: "low",
  });
  await waitFor((m) => m.method === "turn/completed" && m.params?.threadId === threadId);

  // Turn 2: interrupt shortly after it starts
  const t2 = await send("turn/start", {
    threadId,
    input: [{ type: "text", text: "Write the numbers from 1 to 400, one per line, with no other text.", text_elements: [] }],
    effort: "low",
  });
  const turnId = t2.result?.turn?.id;
  await Promise.race([
    waitFor((m) => m.method === "item/agentMessage/delta" && m.params?.turnId === turnId),
    new Promise((r) => setTimeout(r, 4000)),
  ]);
  await send("turn/interrupt", { threadId, turnId });
  await Promise.race([
    waitFor((m) => m.method === "turn/completed" && m.params?.turn?.id === turnId),
    new Promise((r) => setTimeout(r, 15000)),
  ]);
  clearTimeout(hardStop);
  child.stdin.end();
  setTimeout(() => child.kill("SIGTERM"), 1500);
})().catch((e) => {
  out.write(JSON.stringify({ dir: "probe", error: String(e) }) + "\n");
  child.kill("SIGTERM");
});

child.on("exit", (code, sig) => { out.write(JSON.stringify({ dir: "probe", exit: code, signal: sig, t_ms: Date.now() - t0 }) + "\n"); out.end(); });
