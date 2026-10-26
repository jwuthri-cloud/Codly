// Minimal ACP (Agent Client Protocol) v1 client probe.
// Usage: node probe.mjs <outfile.jsonl> <cwd> <wantModel|-> -- <agent-cmd> [args...]
// Logs every JSON-RPC line in both directions as JSONL:
//   {"t": <ms since start>, "dir": "c2a"|"a2c"|"stderr"|"note", "msg"|"text": ...}
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import readline from "node:readline";

const sep = process.argv.indexOf("--");
const [outFile, cwd, wantModel] = process.argv.slice(2, sep);
const [cmd, ...args] = process.argv.slice(sep + 1);
const PROMPT = process.env.PROBE_PROMPT || "Read README.md and reply DONE";
const TIMEOUT_MS = Number(process.env.PROBE_TIMEOUT_MS || 120000);

const out = fs.createWriteStream(outFile);
const t0 = Date.now();
const log = (o) => out.write(JSON.stringify({ t: Date.now() - t0, ...o }) + "\n");
log({ dir: "note", text: `spawn ${cmd} ${args.join(" ")} (cwd=${cwd})` });

const child = spawn(cmd, args, { cwd, stdio: ["pipe", "pipe", "pipe"], env: process.env });
let nextId = 0;
const pending = new Map();

function send(msg) {
  const line = JSON.stringify({ jsonrpc: "2.0", ...msg });
  log({ dir: "c2a", msg: JSON.parse(line) });
  child.stdin.write(line + "\n");
}
function request(method, params) {
  const id = nextId++;
  send({ id, method, params });
  return new Promise((resolve, reject) => pending.set(id, { resolve, reject, method }));
}
const respond = (id, result) => send({ id, result });
const respondError = (id, code, message) => send({ id, error: { code, message } });

// Within-root check so the probe never reads/writes outside the scratch repo.
const inRoot = (p) => path.resolve(p).startsWith(path.resolve(cwd) + path.sep);

async function handleAgentRequest(msg) {
  const { id, method, params } = msg;
  try {
    switch (method) {
      case "session/request_permission": {
        const opts = params.options || [];
        const pick =
          opts.find((o) => o.kind === "allow_once") ||
          opts.find((o) => o.kind === "allow_always") ||
          opts[0];
        return pick
          ? respond(id, { outcome: { outcome: "selected", optionId: pick.optionId } })
          : respond(id, { outcome: { outcome: "cancelled" } });
      }
      case "fs/read_text_file": {
        if (!inRoot(params.path)) return respondError(id, -32602, "path outside workspace");
        let content = fs.readFileSync(params.path, "utf8");
        if (params.line || params.limit) {
          const lines = content.split("\n");
          const start = Math.max(0, (params.line || 1) - 1);
          content = lines.slice(start, params.limit ? start + params.limit : undefined).join("\n");
        }
        return respond(id, { content });
      }
      case "fs/write_text_file": {
        if (!inRoot(params.path)) return respondError(id, -32602, "path outside workspace");
        fs.writeFileSync(params.path, params.content);
        return respond(id, {});
      }
      default:
        return respondError(id, -32601, `Method not found: ${method}`);
    }
  } catch (e) {
    return respondError(id, -32603, String(e?.message || e));
  }
}

readline.createInterface({ input: child.stdout }).on("line", (line) => {
  if (!line.trim()) return;
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return log({ dir: "a2c-nonjson", text: line });
  }
  log({ dir: "a2c", msg });
  if (msg.method && msg.id !== undefined) return handleAgentRequest(msg);
  if (msg.id !== undefined && pending.has(msg.id)) {
    const p = pending.get(msg.id);
    pending.delete(msg.id);
    msg.error ? p.reject(Object.assign(new Error(msg.error.message), { rpc: msg.error })) : p.resolve(msg.result);
  }
});
readline.createInterface({ input: child.stderr }).on("line", (text) => log({ dir: "stderr", text }));
child.on("exit", (code, signal) => {
  log({ dir: "note", text: `agent exited code=${code} signal=${signal}` });
  out.end(() => process.exit(0));
});

const timer = setTimeout(() => {
  log({ dir: "note", text: `TIMEOUT after ${TIMEOUT_MS}ms` });
  child.kill("SIGTERM");
}, TIMEOUT_MS);

(async () => {
  try {
    const init = await request("initialize", {
      protocolVersion: 1,
      clientCapabilities: { fs: { readTextFile: true, writeTextFile: true }, terminal: false },
      clientInfo: { name: "codly-acp-probe", title: "Codly ACP probe", version: "0.0.1" },
    });
    let sess;
    try {
      sess = await request("session/new", { cwd, mcpServers: [] });
    } catch (e) {
      log({ dir: "note", text: `session/new failed: ${JSON.stringify(e.rpc)}; authMethods=${JSON.stringify(init.authMethods)}` });
      throw e;
    }
    const sessionId = sess.sessionId;
    // Pin a cheap model if the agent exposes a model selector.
    const modelOpt = (sess.configOptions || []).find((o) => o.category === "model" || o.id === "model");
    if (wantModel && wantModel !== "-" && modelOpt) {
      const flat = (modelOpt.options || []).flatMap((o) => (o.options ? o.options : [o]));
      const hit = flat.find((o) => o.value === wantModel || String(o.value).endsWith(wantModel));
      if (hit && modelOpt.currentValue !== hit.value) {
        await request("session/set_config_option", { sessionId, configId: modelOpt.id, value: hit.value });
      } else if (!hit) {
        log({ dir: "note", text: `model ${wantModel} not in options; current=${modelOpt.currentValue}` });
      }
    }
    const res = await request("session/prompt", {
      sessionId,
      prompt: [{ type: "text", text: PROMPT }],
    });
    log({ dir: "note", text: `prompt finished stopReason=${res?.stopReason}` });
    if (init.agentCapabilities?.sessionCapabilities?.list) {
      await request("session/list", { cwd }).catch(() => {});
    }
  } catch (e) {
    log({ dir: "note", text: `error: ${e.message}` });
  } finally {
    clearTimeout(timer);
    child.stdin.end();
    setTimeout(() => child.kill("SIGTERM"), 1500);
  }
})();
