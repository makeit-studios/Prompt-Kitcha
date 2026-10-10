const { test, beforeEach } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const ts = require("typescript");

let calls;
let response;
let subscriptions;
const filename = path.resolve(__dirname, "../src/lib/ipc.ts");
const code = ts.transpileModule(fs.readFileSync(filename, "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const exported = {};
vm.runInNewContext(code, {
  exports: exported,
  require: (name) => {
    if (name === "@tauri-apps/api/core") return {
      invoke: async (command, args) => {
        calls.push({ command, args });
        return response;
      },
    };
    if (name === "@tauri-apps/api/event") return {
      listen: async (name, handler) => {
        subscriptions.set(name, handler);
        return () => subscriptions.delete(name);
      },
    };
    throw new Error(`Unexpected IPC dependency: ${name}`);
  },
}, { filename });

beforeEach(() => {
  calls = [];
  response = undefined;
  subscriptions = new Map();
});

function normalizedCalls() {
  return JSON.parse(JSON.stringify(calls));
}

test("capture returns the native structured capture unchanged", async () => {
  response = { capture_id: 41, replaceable: true, text: "Selected request" };
  assert.equal(await exported.ipc.captureSelection(), response);
  assert.deepEqual(normalizedCalls(), [{ command: "capture_selection" }]);
});

test("external-editor session loading and acceptance use the pinned file session ID", async () => {
  response = { session_id: 71, text: "Complete original draft" };
  assert.equal(await exported.ipc.getEditorSession(), response);
  await exported.ipc.saveEditorPrompt("Edited draft\nfinal line", 71);
  assert.deepEqual(normalizedCalls(), [
    { command: "get_editor_session" },
    { command: "save_editor_prompt", args: { text: "Edited draft\nfinal line", sessionId: 71 } },
  ]);
});

test("replacement sends edited text with Tauri's camelCase captureId argument", async () => {
  await exported.ipc.replaceSelection("Edited draft\nsecond line", 41);
  assert.deepEqual(normalizedCalls(), [{
    command: "replace_selection", args: { text: "Edited draft\nsecond line", captureId: 41 },
  }]);
});

test("enhancement forwards the original request, edited previous output and feedback", async () => {
  const req = { text: "Original request", job_id: 123, previous_output: "Edited draft", feedback: "Shorter" };
  response = 123;
  assert.equal(await exported.ipc.optimize(req), 123);
  assert.deepEqual(normalizedCalls(), [{ command: "optimize_text", args: { req } }]);
});

test("cancellation sends the camelCase jobId argument", async () => {
  await exported.ipc.cancel(123);
  assert.deepEqual(normalizedCalls(), [{ command: "cancel_optimize", args: { jobId: 123 } }]);
});

test("event subscriptions use native names and forward only the payload", async () => {
  const cases = [
    ["onSelectionCaptured", "pw:selection-captured", { capture_id: 41, replaceable: true, text: "Selected" }],
    ["onClipboardCaptured", "pw:clipboard-captured", "Copied"],
    ["onCaptureError", "pw:capture-error", "No selection"],
    ["onOptimize", "pw:optimize", { job_id: 123, kind: "done", full_prompt: "Draft" }],
  ];
  for (const [method, name, payload] of cases) {
    let received;
    const unlisten = await exported[method]((value) => { received = value; });
    assert.equal(typeof subscriptions.get(name), "function");
    subscriptions.get(name)({ event: name, id: 99, payload });
    assert.equal(received, payload);
    unlisten();
    assert.equal(subscriptions.has(name), false);
  }
});

test("new BYOK providers keep their own credential and connection command arguments", async () => {
  for (const provider of ["groq", "gemini"]) {
    calls = [];
    await exported.ipc.setApiKey(provider, "mock-key");
    await exported.ipc.hasApiKey(provider);
    await exported.ipc.testConnection(provider, "account-model");
    await exported.ipc.deleteApiKey(provider);
    assert.deepEqual(normalizedCalls(), [
      { command: "set_api_key", args: { provider, key: "mock-key" } },
      { command: "has_api_key", args: { provider } },
      { command: "test_connection", args: { provider, model: "account-model", baseUrl: null } },
      { command: "delete_api_key", args: { provider } },
    ]);
  }
});
