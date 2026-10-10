const { test, beforeEach, afterEach } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const ts = require("typescript");
const { JSDOM } = require("jsdom");

const dom = new JSDOM("<!doctype html><html><body><div id='root'></div></body></html>", { url: "http://localhost" });
for (const name of ["window", "document", "Node", "HTMLElement", "HTMLTextAreaElement", "HTMLInputElement", "Event", "KeyboardEvent"]) {
  globalThis[name] = dom.window[name];
}
Object.defineProperty(globalThis, "navigator", { value: dom.window.navigator, configurable: true });
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

for (const extension of [".ts", ".tsx"]) {
  require.extensions[extension] = (module, filename) => module._compile(ts.transpileModule(fs.readFileSync(filename, "utf8"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2022, esModuleInterop: true },
  }).outputText, filename);
}

let listeners;
let calls;
let generate;
let config;
let configGetter;
let replace;
let clipboardText;
let saveConfig;
let editorSession;
let saveEditor;
let providerList;
let storedKeys;
let keyFailure;
let connectionFailure;
const ipc = {
  getEditorSession: async () => editorSession,
  saveEditorPrompt: async (text, sessionId) => {
    calls.editor.push({ text, sessionId });
    if (saveEditor) await saveEditor();
  },
  quit: async () => calls.quit.push(true),
  getConfig: async () => configGetter ? configGetter() : config,
  optimize: async (request) => {
    calls.optimize.push(request);
    if (generate) generate(request);
    return request.job_id;
  },
  cancel: async (id) => calls.cancel.push(id),
  replaceSelection: async (text, id) => {
    calls.replace.push({ text, id });
    if (replace) await replace();
  },
  writeClipboard: async (text) => calls.copy.push(text),
  hideWindow: async (label) => calls.hide.push(label),
  showWindow: async (label) => calls.show.push(label),
  showPopupAtCursor: async () => {},
  readClipboard: async () => clipboardText,
  listProviders: async () => providerList,
  hasApiKey: async (provider) => storedKeys.has(provider),
  setApiKey: async (provider, key) => {
    calls.keys.push({ provider, key });
    if (keyFailure) throw new Error(keyFailure);
    storedKeys.set(provider, key);
  },
  deleteApiKey: async (provider) => {
    calls.deletedKeys.push(provider);
    storedKeys.delete(provider);
  },
  testConnection: async (provider, model, baseUrl) => {
    calls.connections.push({ provider, model, baseUrl });
    if (connectionFailure) throw new Error(connectionFailure);
    return "ok";
  },
  updateConfig: async (patch) => {
    calls.config.push(patch);
    if (saveConfig) return saveConfig(patch);
    config = { ...config, ...patch };
    return config;
  },
  showSettings: async () => {},
  showImagePrompt: async () => {},
};
const subscribe = (name) => (handler) => {
  listeners.set(name, handler);
  return Promise.resolve(() => listeners.delete(name));
};
const ipcPath = path.resolve(__dirname, "../src/lib/ipc.ts");
require.cache[ipcPath] = { id: ipcPath, filename: ipcPath, loaded: true, exports: {
  ipc,
  onSelectionCaptured: subscribe("selection"),
  onClipboardCaptured: subscribe("clipboard"),
  onCaptureError: subscribe("capture-error"),
  onOptimize: subscribe("optimize"),
} };

const React = require("react");
const { act } = React;
const { createRoot } = require("react-dom/client");
const Popup = require("../src/windows/Popup.tsx").default;
const Settings = require("../src/windows/Settings.tsx").default;
const { IntentBadge } = require("../src/components/IntentBadge.tsx");
const { renderToStaticMarkup } = require("react-dom/server");
let root;

beforeEach(async () => {
  listeners = new Map();
  calls = { optimize: [], replace: [], copy: [], hide: [], show: [], cancel: [], config: [], editor: [], quit: [], keys: [], deletedKeys: [], connections: [] };
  providerList = [];
  storedKeys = new Map();
  keyFailure = null;
  connectionFailure = null;
  editorSession = null;
  saveEditor = null;
  config = { auto_replace: true, max_input_chars: 10000, enhancement_mode: "grounded", provider: "ollama", hotkey: "Ctrl+Shift+Space", onboarded: true };
  clipboardText = "copied source";
  saveConfig = null;
  configGetter = null;
  replace = null;
  generate = (request) => listeners.get("optimize")({
    job_id: request.job_id, kind: "done", full_prompt: "Enhanced draft", intent: "other", mode: "grounded", elapsed_ms: 1,
  });
  root = createRoot(document.getElementById("root"));
  await act(async () => root.render(React.createElement(Popup)));
});

afterEach(async () => {
  await act(async () => root.unmount());
});

async function emit(name, payload) {
  await act(async () => listeners.get(name)(payload));
}

async function capture(id = 41, text = "Original request") {
  await emit("selection", { capture_id: id, text, replaceable: true });
}

function draft() {
  return document.querySelector('textarea[aria-label="Enhanced prompt"]');
}

async function edit(element, text) {
  const prototype = element.tagName === "TEXTAREA" ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  await act(async () => {
    Object.getOwnPropertyDescriptor(prototype, "value").set.call(element, text);
    element.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function key(element, options = {}) {
  const event = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true, ...options });
  await act(async () => element.dispatchEvent(event));
  return event;
}

test("select, enhance, edit, Enter replaces the edited draft with its capture ID", async () => {
  await capture();
  assert.equal(document.activeElement, draft());
  await edit(draft(), "My edited prompt 日本語 🙂\nkeep this line");
  await key(draft());
  assert.deepEqual(calls.replace, [{ text: "My edited prompt 日本語 🙂\nkeep this line", id: 41 }]);
  assert.deepEqual(calls.hide, ["popup"]);
  assert.deepEqual(calls.copy, []);
});

test("Shift+Enter, modifier chords, IME confirmation and repeat never accept", async () => {
  await capture();
  for (const options of [{ shiftKey: true }, { ctrlKey: true }, { altKey: true }, { metaKey: true }, { isComposing: true }, { keyCode: 229 }, { repeat: true }]) {
    const event = await key(draft(), options);
    if (!options.repeat) assert.equal(event.defaultPrevented, false);
  }
  assert.deepEqual(calls.replace, []);
  assert.deepEqual(calls.hide, []);
});

test("empty edited drafts and streaming output cannot be accepted", async () => {
  await capture();
  await edit(draft(), "  \n");
  await key(draft());
  assert.deepEqual(calls.replace, []);
  generate = null;
  await capture(42);
  assert.equal(draft(), null);
  await key(window);
  assert.deepEqual(calls.replace, []);
});

test("clipboard mode accepts edits with Copy & close, never Replace", async () => {
  await emit("clipboard", "Copied source");
  await edit(draft(), "Edited clipboard prompt");
  await key(draft());
  assert.deepEqual(calls.copy, ["Edited clipboard prompt"]);
  assert.deepEqual(calls.replace, []);
  assert.deepEqual(calls.hide, ["popup"]);
});

test("auto-replace disabled stays copy-only and has the correct label", async () => {
  config.auto_replace = false;
  await capture();
  assert.ok(document.body.textContent.includes("Copy & close"));
  await key(draft());
  assert.deepEqual(calls.replace, []);
  assert.deepEqual(calls.copy, ["Enhanced draft"]);
});

test("read-only or unvalidated native captures are copy-only", async () => {
  await emit("selection", { capture_id: 41, text: "Read-only source", replaceable: false });
  assert.ok(document.body.textContent.includes("Copy & close"));
  await key(draft());
  assert.deepEqual(calls.replace, []);
  assert.deepEqual(calls.copy, ["Enhanced draft"]);
});

test("replacement failure keeps the draft editable and available to Copy", async () => {
  await capture();
  await edit(draft(), "Do not lose these edits");
  replace = async () => { throw new Error("Original selection changed"); };
  await key(draft());
  assert.equal(draft().value, "Do not lose these edits");
  assert.equal(draft().readOnly, false);
  assert.match(document.querySelector('[role="alert"]').textContent, /Original selection changed/);
  assert.deepEqual(calls.hide, []);
  assert.deepEqual(calls.show, ["popup"]);
  await act(async () => document.querySelector('button[title="Copy"]').click());
  assert.deepEqual(calls.copy, ["Do not lose these edits"]);
});

test("refinement receives the edited draft; a fresh capture resets old feedback", async () => {
  await capture();
  await edit(draft(), "Edited previous draft");
  const feedback = document.querySelector('textarea[aria-label="Refinement instructions"]');
  await edit(feedback, "Make it shorter");
  await key(feedback);
  assert.equal(calls.optimize.at(-1).previous_output, "Edited previous draft");
  assert.equal(calls.optimize.at(-1).feedback, "Make it shorter");
  assert.equal(calls.replace.length, 0);
  await edit(feedback, "Old unsent instructions");
  feedback.focus();
  await capture(42, "Another source");
  assert.equal(document.activeElement, draft());
  assert.equal(document.querySelector('textarea[aria-label="Refinement instructions"]').value, "");
  assert.equal(calls.optimize.at(-1).previous_output, null);
  assert.equal(calls.optimize.at(-1).feedback, null);
});

test("double Enter cannot start duplicate replacement while apply is pending", async () => {
  await capture();
  let finishReplace;
  replace = () => new Promise((resolve) => { finishReplace = resolve; });
  await key(draft());
  await key(draft());
  assert.equal(calls.replace.length, 1);
  assert.equal(draft().readOnly, true);
  await act(async () => finishReplace());
  assert.deepEqual(calls.hide, ["popup"]);
});

test("newer capture invalidates acceptance waiting for configuration", async () => {
  await capture();
  let resolveConfig;
  configGetter = () => new Promise((resolve) => { resolveConfig = resolve; });
  await key(draft());
  const resolveOldAcceptance = resolveConfig;
  configGetter = null;
  await capture(42, "New prompt");
  await act(async () => resolveOldAcceptance(config));
  assert.deepEqual(calls.replace, []);
  assert.deepEqual(calls.hide, []);
  assert.equal(draft().value, "Enhanced draft");
  await key(draft());
  assert.equal(calls.replace.at(-1).id, 42);
});

test("Escape cancels without copying or replacing; stale job output is ignored", async () => {
  generate = null;
  await capture();
  const id = calls.optimize.at(-1).job_id;
  await key(window, { key: "Escape" });
  assert.ok(calls.cancel.includes(id));
  await emit("optimize", { job_id: id, kind: "done", full_prompt: "Stale draft", intent: "other", mode: "ai", elapsed_ms: 1 });
  assert.equal(draft(), null);
  assert.deepEqual(calls.replace, []);
  assert.deepEqual(calls.copy, []);
});

test("unexpected and prototype intent names safely render as Prompt", () => {
  for (const intent of ["unexpected_model_intent", "__proto__", "constructor"]) {
    assert.match(renderToStaticMarkup(React.createElement(IntentBadge, { intent })), /Prompt/);
  }
});

const longPrompt = () => Array.from({ length: 100 }, (_, i) => `  Line ${i + 1}: 日本語 🙂\t${"preserve this requirement, URL https://example.com/path and code(); ".repeat(3)}`).join("\n") + "\nFINAL_SENTINEL";
const button = (text) => [...document.querySelectorAll("button")].find((element) => element.textContent === text);
const sourceBox = () => document.querySelector('textarea[aria-label="Prompt to enhance"]');
const feedbackBox = () => document.querySelector('textarea[aria-label="Refinement instructions"]');

test("manual input keeps over 100 lines and over 10000 characters through enhance and copy", async () => {
  const text = longPrompt();
  assert.ok(text.length > 10000);
  await edit(sourceBox(), text);
  assert.equal(sourceBox().value, text);
  assert.equal(sourceBox().hasAttribute("maxlength"), false);
  assert.match(document.body.textContent, /101 lines/);
  generate = (request) => listeners.get("optimize")({ job_id: request.job_id, kind: "done", full_prompt: request.text });
  await key(sourceBox());
  assert.equal(calls.optimize.at(-1).text, text);
  assert.equal(draft().value, text);
  await key(draft());
  assert.deepEqual(calls.copy, [text]);
  assert.deepEqual(calls.replace, []);
});

test("Paste from clipboard loads the complete long prompt without enhancing prematurely", async () => {
  clipboardText = longPrompt();
  await act(async () => button("Paste from clipboard").click());
  assert.equal(sourceBox().value, clipboardText);
  assert.equal(document.activeElement, sourceBox());
  assert.deepEqual(calls.optimize, []);
  await act(async () => button("Enhance").click());
  assert.equal(calls.optimize.at(-1).text, clipboardText);
});

test("Use copied text forwards all lines after a terminal capture failure", async () => {
  clipboardText = longPrompt().replaceAll("\n", "\r\n");
  await emit("capture-error", "Terminal selection is not editable; copy manually.");
  await act(async () => button(" Use copied text").click());
  assert.equal(calls.optimize.at(-1).text, clipboardText);
  await key(draft());
  assert.deepEqual(calls.replace, []);
  assert.deepEqual(calls.copy, ["Enhanced draft"]);
});

test("manual source modifier chords, IME, and repeat do not enhance", async () => {
  await edit(sourceBox(), "Source\nsecond line");
  for (const options of [{ shiftKey: true }, { ctrlKey: true }, { altKey: true }, { metaKey: true }, { isComposing: true }, { keyCode: 229 }, { repeat: true }]) {
    await key(sourceBox(), options);
  }
  assert.deepEqual(calls.optimize, []);
});

test("multiline refinement is forwarded whole; Shift+Enter does not submit", async () => {
  await capture();
  const text = longPrompt();
  await edit(feedbackBox(), text);
  assert.equal(feedbackBox().value, text);
  for (const options of [{ shiftKey: true }, { ctrlKey: true }, { altKey: true }, { metaKey: true }, { isComposing: true }, { repeat: true }]) {
    await key(feedbackBox(), options);
  }
  assert.equal(calls.optimize.length, 1);
  await key(feedbackBox());
  assert.equal(calls.optimize.at(-1).feedback, text);
  assert.deepEqual(calls.replace, []);
});

test("a limit rejection retains the entire original source for editing and retry", async () => {
  const text = longPrompt();
  generate = () => { throw new Error("This request exceeds the 10000 input limit; no requirements were truncated."); };
  await edit(sourceBox(), text);
  await key(sourceBox());
  assert.match(document.querySelector('[role="alert"]').textContent, /10000 input limit/);
  await act(async () => button("Edit source").click());
  assert.equal(sourceBox().value, text);
  assert.deepEqual(calls.replace, []);
  generate = (request) => listeners.get("optimize")({ job_id: request.job_id, kind: "done", full_prompt: request.text });
  await key(sourceBox());
  await key(draft());
  assert.deepEqual(calls.copy, [text]);
});

test("editing source cannot reuse the previous native replacement target", async () => {
  await capture();
  await act(async () => button("Edit source").click());
  await edit(sourceBox(), longPrompt());
  await key(sourceBox());
  await key(draft());
  assert.deepEqual(calls.replace, []);
  assert.deepEqual(calls.copy, ["Enhanced draft"]);
});

test("failed refinement preserves multiline instructions for recovery", async () => {
  await capture();
  const text = longPrompt();
  await edit(feedbackBox(), text);
  generate = () => { throw new Error("Feedback exceeds the configured limit"); };
  await key(feedbackBox());
  assert.equal(feedbackBox().value, text);
  assert.match(document.querySelector('[role="alert"]').textContent, /Feedback exceeds/);
});

test("Settings explicitly saves a larger limit and rejects invalid values", async () => {
  await act(async () => root.render(React.createElement(Settings)));
  const input = document.getElementById("max-input-chars");
  assert.equal(input.value, "10000");
  await edit(input, "100000");
  assert.deepEqual(calls.config, []);
  await act(async () => button("Save limit").click());
  assert.deepEqual(calls.config, [{ max_input_chars: 100000 }]);
  await edit(input, "0");
  await act(async () => button("Save limit").click());
  assert.match(document.querySelector('[role="alert"]').textContent, /positive whole number/);
  assert.equal(calls.config.length, 1);
});

test("a settings save failure is visible and does not change the configured limit", async () => {
  await act(async () => root.render(React.createElement(Settings)));
  saveConfig = async () => { throw new Error("Config write failed"); };
  await edit(document.getElementById("max-input-chars"), "100000");
  await act(async () => button("Save limit").click());
  assert.match(document.querySelector('[role="alert"]').textContent, /Config write failed/);
  assert.equal(config.max_input_chars, 10000);
});

async function openEditor(text = longPrompt()) {
  await act(async () => root.unmount());
  editorSession = { session_id: 71, text };
  root = createRoot(document.getElementById("root"));
  await act(async () => root.render(React.createElement(Popup)));
}

test("external-editor Enter returns the complete edited draft and exits without clipboard or native input", async () => {
  await openEditor();
  assert.equal(calls.optimize.at(-1).text, longPrompt());
  assert.ok(button("Return to OpenCode"));
  const text = longPrompt() + "\nMy final edits";
  await edit(draft(), text);
  await key(draft(), { shiftKey: true });
  assert.deepEqual(calls.editor, []);
  await key(draft());
  assert.deepEqual(calls.editor, [{ text, sessionId: 71 }]);
  assert.deepEqual(calls.quit, [true]);
  assert.deepEqual(calls.replace, []);
  assert.deepEqual(calls.copy, []);
  assert.deepEqual(calls.hide, []);
});

test("external-editor Escape exits without writing the draft", async () => {
  await openEditor();
  await key(draft(), { key: "Escape" });
  assert.deepEqual(calls.editor, []);
  assert.deepEqual(calls.quit, [true]);
  assert.deepEqual(calls.copy, []);
  assert.deepEqual(calls.replace, []);
});

test("external-editor file save failures keep the draft editable and do not exit", async () => {
  await openEditor();
  saveEditor = async () => { throw new Error("The original editor draft changed"); };
  await edit(draft(), "Keep my edits after failure");
  await key(draft());
  assert.equal(draft().value, "Keep my edits after failure");
  assert.equal(draft().readOnly, false);
  assert.match(document.querySelector('[role="alert"]').textContent, /original editor draft changed/);
  assert.deepEqual(calls.quit, []);
});

test("external-editor mode works with auto-replace off and retains its target through Edit source", async () => {
  config.auto_replace = false;
  await openEditor();
  await act(async () => button("Edit source").click());
  await edit(sourceBox(), "Revised original prompt");
  await key(sourceBox());
  assert.ok(button("Return to OpenCode"));
  await key(draft());
  assert.deepEqual(calls.editor, [{ text: "Enhanced draft", sessionId: 71 }]);
  assert.deepEqual(calls.copy, []);
});

test("empty external-editor drafts remain multiline-editable and can be enhanced", async () => {
  await openEditor("");
  assert.ok(sourceBox());
  await edit(sourceBox(), longPrompt());
  await key(sourceBox());
  await key(draft());
  assert.equal(calls.optimize.at(-1).text, longPrompt());
  assert.deepEqual(calls.editor, [{ text: "Enhanced draft", sessionId: 71 }]);
});

test("duplicate external-editor Enter cannot start two file writes", async () => {
  await openEditor();
  let finish;
  saveEditor = () => new Promise((resolve) => { finish = resolve; });
  await key(draft());
  await key(draft());
  assert.equal(calls.editor.length, 1);
  assert.equal(draft().readOnly, true);
  await act(async () => finish());
  assert.deepEqual(calls.quit, [true]);
});

async function providerSettings() {
  providerList = [
    { id: "groq", label: "Groq", needs_key: true, default_model: "openai/gpt-oss-20b", key_help_url: "https://console.groq.com/keys" },
    { id: "gemini", label: "Gemini (Google AI Studio)", needs_key: true, default_model: "gemini-3.5-flash-lite", key_help_url: "https://aistudio.google.com/apikey" },
  ];
  config.base_url = "https://previous-provider.example/v1";
  await act(async () => root.render(React.createElement(Settings)));
  await act(async () => button("Provider & API").click());
}

async function selectProvider(entry) {
  const card = [...document.querySelectorAll("button")].find((b) => b.querySelector(".font-semibold")?.textContent === entry.label);
  await act(async () => card.click());
}

test("Groq and Gemini selections reset endpoint/model, use distinct keys, and test the selected model", async () => {
  await providerSettings();
  for (const entry of providerList) {
    await selectProvider(entry);
    assert.deepEqual(calls.config.at(-1), { provider: entry.id, model: entry.default_model, base_url: null });
    assert.equal(document.querySelector('[aria-label="Provider model"]').value, entry.default_model);
    assert.match(document.body.textContent, /BYOK/);
    await edit(document.querySelector('[aria-label="Provider API key"]'), `  mock-${entry.id}-key  `);
    await act(async () => button("Save Key").click());
    assert.deepEqual(calls.keys.at(-1), { provider: entry.id, key: `mock-${entry.id}-key` });
    assert.match(document.body.textContent, /API Key Stored/);
    await edit(document.querySelector('[aria-label="Provider model"]'), "my-account-model");
    await act(async () => [...document.querySelectorAll("button")].find((b) => b.textContent.includes("Test Connection")).click());
    assert.deepEqual(calls.connections.at(-1), { provider: entry.id, model: "my-account-model", baseUrl: undefined });
    assert.match(document.body.textContent, /Connected/);
  }
  assert.equal(storedKeys.size, 2);
  await act(async () => [...document.querySelectorAll("button")].find((b) => b.textContent.trim() === "Remove").click());
  assert.deepEqual(calls.deletedKeys, ["gemini"]);
  assert.equal(storedKeys.has("groq"), true);
  assert.equal(storedKeys.has("gemini"), false);
});

test("provider key-save and connection failures stay visible without losing unsaved input", async () => {
  await providerSettings();
  await selectProvider(providerList[0]);
  const input = document.querySelector('[aria-label="Provider API key"]');
  await edit(input, "mock-unsaved-key");
  keyFailure = "Windows Credential Manager unavailable";
  await act(async () => button("Save Key").click());
  assert.equal(input.value, "mock-unsaved-key");
  assert.match(document.querySelector('[role="alert"]').textContent, /Credential Manager/);
  assert.equal(storedKeys.size, 0);
  keyFailure = null;
  await act(async () => button("Save Key").click());
  connectionFailure = "429: provider quota exceeded";
  await act(async () => [...document.querySelectorAll("button")].find((b) => b.textContent.includes("Test Connection")).click());
  assert.match(document.querySelector('[role="alert"]').textContent, /429.*quota/);
});
