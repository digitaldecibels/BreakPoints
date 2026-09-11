#!/usr/bin/env node
// A stdio MCP transport that forwards to the running Break/Points HTTP bridge.
//
// Claude Code speaks HTTP and does not need this. Codex and anything else that
// launches a process and talks over stdin and stdout does.
//
//   [mcp_servers.breakpoints]
//   command = "node"
//   args = ["/absolute/path/to/BreakPoints/agent/stdio-shim.js"]
//
// The token is read from the app's own config, so there is nothing to copy by
// hand. Set BREAKPOINTS_TOKEN or BREAKPOINTS_URL to override either.

import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { createInterface } from "node:readline";

const URL_BASE = process.env.BREAKPOINTS_URL ?? "http://127.0.0.1:7333";

function readToken() {
  if (process.env.BREAKPOINTS_TOKEN) return process.env.BREAKPOINTS_TOKEN;
  const config = join(
    homedir(),
    "Library",
    "Application Support",
    "com.digitaldecibels.breakpoints",
    "breakpoints.json"
  );
  try {
    return JSON.parse(readFileSync(config, "utf8")).bridgeToken ?? null;
  } catch {
    return null;
  }
}

const token = readToken();

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

/** A transport error still has to look like a JSON-RPC reply, or the client hangs. */
function failure(id, message) {
  send({ jsonrpc: "2.0", id: id ?? null, error: { code: -32000, message } });
}

const lines = createInterface({ input: process.stdin });

lines.on("line", async (line) => {
  const text = line.trim();
  if (!text) return;

  let request;
  try {
    request = JSON.parse(text);
  } catch {
    failure(null, "the shim was sent something that is not JSON");
    return;
  }

  try {
    const response = await fetch(`${URL_BASE}/mcp`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...(token ? { authorization: `Bearer ${token}` } : {}),
      },
      body: JSON.stringify(request),
    });
    const body = await response.json();
    // A notification has no id and expects no reply.
    if (request.id !== undefined) send(body);
  } catch (error) {
    failure(
      request.id,
      `Break/Points is not answering on ${URL_BASE}. Open the app and turn the agent bridge on in Settings. (${error.message})`
    );
  }
});
