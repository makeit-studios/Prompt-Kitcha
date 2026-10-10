# Windows prompt-enhancer fork

This fork targets Windows 10/11. The workflow is select text, press
`Ctrl+Shift+Space`, edit the enhanced prompt, then press Enter to Replace or Copy.
There is no subscription or required hosted service.

## Edit and accept

After enhancement finishes, the popup focuses an editable prompt field. Edit the
final wording directly. Enter in that field accepts the edited draft; Shift+Enter
inserts a newline. Escape cancels without replacing. Enter in the separate refine
box regenerates using its instructions. AI mode also receives the edited draft;
Instant mode currently rebuilds from the original source and latest instructions,
so it does not retain manual edits or earlier refinement instructions on regenerate.

With Auto-replace enabled and a validated editable capture, Enter replaces the
original selection. With Auto-replace disabled, copied-text input, or an
unvalidated/read-only capture, Enter copies the result and closes instead. The
button and keyboard hint show which action will happen. Partial, empty, failed,
or still-streaming output cannot be accepted. Replacement errors keep the edited
draft visible and available to Copy.

The app waits for Enter and shortcut modifiers to be released before refocusing
the target. It never injects Enter into the target application. Double Enter
cannot start duplicate replacement. Every capture has an ID so an older draft
cannot replace a newer capture; new input resets old feedback and draft state.
The tray and floating pill are clipboard-only entry points and cannot reuse an
old replaceable selection.

## Multiline prompts and clipboard input

The empty popup now has a scrollable **Prompt to enhance** textarea. Paste with
Ctrl+V or choose **Paste from clipboard**, review the line/character count, then
press Enter or **Enhance**. Shift+Enter inserts a newline. There is no line-count
or HTML maxlength cap; a 100-line prompt does not have to fit on screen.
The refinement field is also a multiline textarea; Enter refines and Shift+Enter
adds a line. Modifier chords, repeated Enter, and IME confirmation do not submit.

Settings > General > **Maximum input characters** exposes the existing backend
limit. The default and this machine's previously saved value are 10000. For long
prompts, set it above the complete character count, for example **100000**, and
choose **Save limit**. Existing settings are not silently changed. The prompt and
refinement text are forwarded whole; exceeding the configured limit produces an
error rather than silently cutting text. AI providers still have their own model
context/output limits; increasing this setting does not remove those limits.
This section concerns the ordinary prompt enhancer, not Image Prompt Maker.

On an enhancement error, **Edit source** restores the full original request for
editing/retry. It intentionally drops any previous native replacement target;
manually edited source is copy-only. Failed refinements retain their instructions.
After an OpenCode capture error, use the multiline entry area or **Use copied
text**, then accept with **Copy & close** and paste manually into OpenCode's
prompt input. This fixes the PromptKitcha input path, not terminal auto-replacement.

Manual check: open `tests/fixtures/multiline-paste-101-lines.txt` in an editor,
copy all text, paste into **Prompt to enhance**, and scroll to `FINAL_SENTINEL`.
Set the character limit above the displayed count and choose Instant grounded.
Enhance, confirm the Context contains every numbered line and the sentinel, then
press Enter to Copy & close. Paste into an unsaved Notepad document to inspect the
complete result, then test pasting into OpenCode's prompt input (not a shell).
The fixture has 101 nonempty lines; a final file newline may add a blank line.
User-reported evidence: Notepad selection/Enter works and OpenCode copying reaches
Windows clipboard. This new multiline build has not been live-GUI verified here.

## Enhancement modes

- **Instant grounded** is the default. It runs locally, without an API key,
  internet connection, or language model. It wraps the complete original request
  in Task, Context, Constraints, and Output sections. It does not correct spelling
  or paraphrase the source. The complete source, including leading/trailing
  whitespace, indentation, Unicode, code, paths, and requirements, is retained.
- **AI enhanced** corrects spelling and improves wording. Choose Ollama for local
  inference with no API key, or OpenAI, Anthropic, OpenRouter, Groq, or Gemini with your own key.
  Hosted provider usage may cost money. Selected text, refinement instructions,
  and the previous draft are sent to the chosen provider; active window titles
  are not included. Keys remain in Windows Credential Manager.

AI mode uses one model call, a compact enhancement-only system instruction,
low temperature, and an output budget scaled to the source length. It never
deliberately answers the request or turns a diagnosis/review into implementation.
The application validates JSON and the four nonempty sections, but cannot
guarantee semantic accuracy or that every literal survived an AI rewrite. Review
before replacing; use grounded mode when exact source preservation matters.

For Ollama, start its local server, select Ollama in Settings, set the installed
model name, and use `http://localhost:11434` as the base URL. Models are not
downloaded automatically. JSON output is requested, thinking output is disabled,
and the model is kept loaded for 15 minutes. These are supported by the
[Ollama chat API](https://docs.ollama.com/api/chat).

### Groq and Gemini BYOK setup

Settings > Provider & API now lists Groq and Gemini alongside the existing providers.
Select the provider, save its own key, and use Test Connection. Under General,
select AI enhanced to use it; Instant grounded still performs no API request.
Selecting a provider resets the model to its default and clears a previous custom
base URL. Model names remain editable to match the models available to your account.

| Provider | Default model | Built-in API base | Key source |
| --- | --- | --- | --- |
| Groq | `openai/gpt-oss-20b` | `https://api.groq.com/openai/v1` | [Groq console](https://console.groq.com/keys) |
| Gemini | `gemini-3.5-flash-lite` | `https://generativelanguage.googleapis.com/v1beta/openai` | [Google AI Studio](https://aistudio.google.com/apikey) |

These use the documented [Groq OpenAI-compatible API](https://console.groq.com/docs/openai)
and [Gemini OpenAI-compatible API](https://ai.google.dev/gemini-api/docs/openai).
No OpenAI account, new SDK, or dependency is needed for these two providers.
The Groq model's `openai/` prefix is its model ID; the request still goes to Groq.
Gemini uses AI Studio API keys, not Vertex AI credentials. Keys occupy separate
`groq_api_key` and `gemini_api_key` entries in Windows Credential Manager, not
config.toml. Each key is sent as authentication only to its selected API endpoint;
prompt text is sent to that provider. A custom base URL changes the destination.

Groq uses `max_completion_tokens`; Gemini uses `max_tokens`. For the default
reasoning models, Groq requests low effort and Gemini requests minimal effort.
Model context, thinking, output limits, account quotas, and availability still
apply. The connection test allows a 1024-token budget for these providers and has
a 30-second timeout; it rejects empty output. It is a real model request and can
consume quota or incur usage charges. Free-tier access is provider-controlled,
not an unlimited-free promise by this app. Hosted testing requires your own key.

HTTP failures, streamed API errors, and length/content-filter stops fail visibly;
the partial enhancement cannot be accepted. HTTP error text redacts the submitted
key. Saving/removing keys and failed connection tests show their errors in Settings.
Offline verification uses dummy keys and local mock HTTP servers. An opt-in
live Gemini backend test also passed; see Build and verification below.
Real Groq and Windows credential-storage/UI acceptance still require a check.

## Selection and replacement safety

Windows UI Automation is tried first. Password fields and discontiguous
selections are rejected. Clipboard capture is allowed only for Notepad, WordPad,
and Write when native selection is unavailable. No blind Ctrl+C fallback is
used in VS Code, browsers, terminals, Electron, or unknown applications.

In VS Code, set `"editor.accessibilitySupport": "on"` if the selection is not
exposed. This is a suggested compatibility step, not a guarantee; integrated
terminals, custom webviews, protected/elevated applications, and some controls
may not expose a usable selection. Cross-application coverage requires manual
testing. Unsupported selections fail visibly instead of guessing from stale
clipboard contents.
See [VS Code's accessibility documentation](https://code.visualstudio.com/docs/configure/accessibility/accessibility)
for this setting and its screen-reader-mode tradeoffs.

Replace refocuses the original window and checks the captured text, focused
control identity, document fingerprint, and selection position before Paste.
UI Automation replacement also requires the selection's read-only attribute to
confirm it is editable. The native-editor clipboard fallback additionally checks
the focused Edit/RichEdit control, range endpoints, and document fingerprint;
if these cannot be confirmed, the result is copy-only. Read-only or unknown controls
require Copy and manual Paste. It refuses a changed selection and checks whether
Windows accepted keyboard input. It preserves the previous plain-text clipboard
when possible, but not rich-text/custom clipboard formats. Non-text clipboard
contents block Replace. Clipboard restoration is best-effort; applications that
read the clipboard unusually late can still require Copy and manual Paste.
The explicit Copy button intentionally overwrites the clipboard. The copied-text
pill has no verified original selection, so use Copy for results opened that way.

## VS Code terminal / OpenCode

Terminal screen selection is not necessarily exposed as an editable Windows
selection. Use the clipboard workflow instead of automatic replacement:

1. Hold Alt while dragging if OpenCode owns mouse selection, then copy the
   selected prompt text yourself using VS Code's
   terminal Copy command. Confirm it pastes correctly into Notepad if uncertain;
   an internal OpenCode selection alone might not update the Windows clipboard.
2. Click the Optimize pill. If it does not appear, press `Ctrl+Shift+Space` and
   choose **Use copied text** when native capture fails.
3. Review the enhanced result, choose **Copy & close**, and manually paste into
   OpenCode's prompt input. Clear the old prompt yourself if needed. Never paste
   the structured result at a shell command prompt.

Enable Settings > General > Show pill on copy for automatic clipboard offers.
The watcher checks Windows clipboard sequence numbers every 200 ms, so copying
identical text again is detected. Our own clipboard writes are suppressed, a
hidden pill is re-shown, and clicking the pill reads the current clipboard rather
than an older cached copy. Copied-text mode disables automatic replacement and
never sends Ctrl+C into the terminal. A read-only native terminal selection also
still requires Copy and manual Paste.

VS Code's [terminal basics](https://code.visualstudio.com/docs/terminal/basics#copy--paste)
documents Windows copy/paste keys and `terminal.integrated.copyOnSelection`.
If the terminal application owns mouse events, holding Alt allows VS Code's
normal terminal selection, as described in that page's Using the mouse section.
OpenCode also supports [`/editor`](https://opencode.ai/docs/tui/#editor) to compose
messages in an external editor. For a VS Code editor workflow, set
`$env:EDITOR = "code --wait"` in PowerShell before starting OpenCode, then use
`/editor`. This changes only that PowerShell session and its child processes.

Terminal/OpenCode GUI acceptance remains a manual check; unit tests cannot prove
that a specific terminal configuration copies text to the Windows clipboard.

### OpenCode draft handoff (external editor)

The installed CLI is `@opencode/cli` 2.0.16, not the older v1 TUI. Its inspected
2.0.16 plugin context does not expose composer-text getters/setters; the old
`/tui/append-prompt` and `/tui/clear-prompt` workflow is not a verified v2 bridge.
The live server's `/api/health` rejects an unauthenticated request with HTTP 401.
No server token was printed, no prompt was cleared, and no message was submitted.

This fork instead supports OpenCode's documented external-editor hook. It returns
the **whole current draft**, not a mouse-selected terminal screen range. In a
PowerShell terminal before launching/relaunching OpenCode, run:

```powershell
& 'C:\Users\Karikari\OneDrive\Documents\Projects\Prompt-Kitcha\scripts\Enable-OpenCodeEditor.ps1'
opencode --continue
```

The script points `VISUAL` and `EDITOR` at this repo's rebuilt release executable
with `--opencode-editor`. These environment values apply only to this PowerShell
process and programs subsequently launched from it; an already-running OpenCode
does not inherit them. It does not alter Windows-wide variables or OpenCode
config files. Both editor variables are set because VISUAL can take precedence.
Use a fresh terminal for your previous editor environment.

1. Type/paste your draft in OpenCode's prompt input, without submitting it.
2. Press **Ctrl+X, then E** (or `/editor`). PromptKitcha opens with that draft.
3. Enhance/review/edit; Enter on the enhanced draft selects **Return to OpenCode**.
   Shift+Enter adds a line. Escape or closing the popup leaves the file unchanged.
4. OpenCode reads the returned draft into its composer. Review there, then press
   OpenCode's own Enter only when you intend to send it.

The handoff uses the supplied UTF-8 draft file, never SendInput, terminal Ctrl+C,
clipboard overwrite, session-prompt API calls, or a simulated submit. The editor
waits for Enter and shortcut modifiers to be released before accepting and
exiting, with a visible error if they remain held. This avoids returning focus
to OpenCode while the accept key is still down; held-key GUI acceptance still
requires a manual check.
The editor
process does not register the global hotkey or start the tray/clipboard watcher,
so the regular PromptKitcha instance can stay running. It checks the original
file contents before returning, stages/syncs the entire result in the same folder,
then renames it into place. Changed/deleted files and stale/duplicate session IDs
are rejected; file-save errors keep the edited result available to Copy. This
is not a cross-process transactional lock: a concurrent writer between the final
check and rename remains a race. Use the OpenCode-owned temporary draft, not a
shared project document. Editor files/results are capped at 4 MiB; enhancement's
configured character limit and AI provider limits also still apply.

Source: [OpenCode v2 TUI editor](https://opencode.ai/v2/docs/cli/tui/) and
[v2 CLI plugin API](https://opencode.ai/v2/docs/build/plugins/cli/).
This is an implemented integration with file-I/O and mocked-IPC tests, not a
live OpenCode GUI acceptance result. The original global shortcut still cannot
replace an unvalidated terminal screen selection. Other terminal apps continue
to use Copy & close/manual Paste unless they provide their own integration.

## Startup and responsiveness

Settings > General > Start with Windows opts into startup; it is off by default.
Changing this setting registers/unregisters the current executable. Test startup
with an installed build, not a temporary development binary.

Native selection and keystrokes run on blocking worker threads. AI jobs can be
cancelled, stale job events are ignored, and a 120-second timeout covers both
connection and streaming. Incomplete/error results cannot be replaced. Ollama
stream decoding preserves UTF-8 split across network packets and final content.
JSON strings are decoded incrementally rather than reparsing the entire growing
prompt for every token.

The popup displays backend enhancement elapsed time. It excludes selection
capture, UI rendering, replacement, and full hotkey-to-result latency. No measured
end-to-end latency claim is made yet.

## Build and verification

As of October 8, 2026, required Windows build dependencies are installed and
dependency downloads have succeeded. Verified versions: Visual Studio Build
Tools 2022 17.14.41, MSVC 14.44.35207, Windows SDK 10.0.26100.0, WebView2
154.0.4258.53, Rust/Cargo 1.99.0, Node 22.22.3, and npm 10.9.8. Rust Tauri and
the JavaScript API/CLI are aligned at 2.12.1. No restart is required by Build Tools.

The October 9 provider frontend production build passes. Thirty
frontend regressions exercise the popup and Settings with mocked IPC: edited draft acceptance,
keyboard/IME protection, streaming/empty output, clipboard/read-only modes,
failed replacement recovery, refinement input, consecutive prompt focus,
duplicate Enter, stale capture/job handling, and unknown intent fallback.
These tests do not exercise Windows capture or Paste.
Seven IPC-wrapper contract tests additionally check the structured capture,
camelCase `captureId` and `jobId` command arguments, edited refinement input,
editor-session command arguments, and native event names/payload forwarding.
`npm test` now passes 37 tests total, including new provider selection, default
model/base reset, separate key saving/removal, selected-model connection arguments,
and visible credential-save/quota failures.
These tests mock the Tauri SDK boundary; they do not prove native command
execution. New tests cover a 101-line source over 10000 characters, multiline
refinement, direct clipboard input including CRLF, keyboard protection, full-source
recovery after a limit rejection, stale replacement-target removal, and explicit
limit saving/validation/failure. Large-input tests mock enhancement; they verify
unmodified input forwarding, not that a configured backend/model accepts it.
The locked offline Rust suite was rerun successfully after this change.
The editor tests cover full-draft loading/acceptance, no clipboard/native-input
calls, Escape cancellation, source editing, empty drafts, save failures, duplicate
Enter protection, and the file-session IPC contract. The process-scoped setup
script was executed in an isolated shell and produced the expected quoted
executable command; it did not launch OpenCode or change the running TUI.
The full locked, offline Rust suite passes:
47 tests passed, zero failed, and two opt-in tests (benchmark and live Gemini)
were ignored by the normal offline run.
The provider tests use dummy keys and local HTTP servers to verify Groq and
Gemini request authentication, complete multiline Unicode streaming, model
budgets, key redaction, and visible quota, malformed-response, and truncation
errors. They do not verify real provider accounts or live API requests.

An additional ignored test, `live_gemini_multiline_enhancement_pipeline`, is
opt-in and uses the actual Gemini transport, streaming JSON extractor, and
prompt validator and source guard with synthetic 100-line input. It checks the
entire original input verbatim, all 96 unchanged code lines in their original
order, path/URL/Unicode literals, the review-only constraint, and equality of
streamed and parsed model text before source guarding. It does not exercise
the popup, Credential Manager, or Enter
replacement. The key file may contain repeated copies of one distinct `AIza`
or `AQ.` key on separate lines. No key value is printed or stored by the test.
Running this test contacts Google and may consume quota or incur usage charges:

```powershell
$env:PROMPTKITCHA_GEMINI_KEY_FILE = 'C:\path\Gemini api key.txt'
cargo test --offline --locked --manifest-path src-tauri/Cargo.toml live_gemini_multiline_enhancement_pipeline -- --ignored --nocapture
```

The optional `PROMPTKITCHA_GEMINI_TEST_MODEL` environment variable overrides the
default Gemini model for this test only. Use the MSVC developer environment
described above. Normal test runs skip this live test.

Live Gemini verification passed on October 10 using a user-supplied key without
printing or persisting it. The `gemini-3.5-flash-lite` backend pipeline processed
100 input lines (1,945 characters), preserved all 96 code lines and the entire
source, and produced 2,242 final characters. Six model-text chunks were parsed;
the API request, parsing, validation, and source guarding took 1,422 ms in this
single test, not a benchmark. The key was used only in memory. Groq has not been
live-tested, and native popup/Enter/Credential Manager verification remains
unconfirmed because the computer-use helper was unavailable.

Gemini enhancement requests now send an explicit JSON schema; Groq enhancement
requests use JSON object mode, which retains streaming support. Connection
tests request plain text instead. Application validation still rejects invalid
prompt sections and incomplete responses. JSON mode alone does not guarantee
semantic faithfulness: live investigation caught changes to long model-copied
code. Final AI-enhanced Context is now rebuilt from the original input and
feedback verbatim, inside dynamically sized code fences, rather than relying
on the model to reproduce them. Task, Constraints, and Output remain AI-written
and should still be reviewed. The streaming preview can differ from the final
Context when the complete source is restored on completion. Instant grounded
mode remains the fully local, deterministic option.
Six native editor tests additionally check argument validation, a real 100+ line
UTF-8/CRLF file round trip, unchanged files on cancellation, changed/deleted input,
invalid/missing input, duplicate acceptance, and staging-file cleanup.
Four additional regression tests cover repeated identical copies, unchanged
clipboard state, empty copies, and suppressing only our own clipboard sequence.
The optimized standalone renderer benchmark also passes. Cross-application GUI
acceptance and full hotkey-to-result latency remain unverified. The computer-use
skill's native helper could not connect for the October 9 Notepad check; no native
GUI acceptance result is claimed. Ollama and models
are optional and have not been installed; instant mode needs neither. A WASI
target is not needed for this Windows application. npm reported 14 dependency
vulnerabilities (1 low, 3 moderate, 10 high); remediation remains separate from
this feature change.

The executable and NSIS installer were rebuilt successfully on October 10 with
the multiline input, editable Enter workflow, Groq/Gemini providers, structured
AI output, and literal-source preservation in the final Context.
The executable embeds `popup-CC7zbBVR.js` and `settings-DEltvmnt.js`, matching
the final frontend build. Both provider endpoints and default models were
confirmed in the release executable, along with the source-preservation marker.
Artifact locations:

- `src-tauri/target/release/promptkitcha.exe` (6,672,384 bytes).
- `src-tauri/target/release/bundle/nsis/PromptKitcha_0.1.0_x64-setup.exe`
  (2,419,343 bytes).

Installer SHA-256:
`F104444BDCDFD7A7C6E6DF0F8462482BCFF5F2D1CE4D4E90ED2233054E373959`.
Quit older running builds before launching or installing this rebuild. The
product version is still 0.1.0; the October 8 installer lacks these changes.

Run the setup file for an installed build. The build has not been installed or
GUI-tested here; verify selection and replacement manually before relying on it
across applications. This is an unsigned local build, not a signed release.

Install Node.js 22.22.2 or newer for the DOM tests, Rust 1.90 or newer,
Microsoft C++ Build Tools with a Windows SDK,
and WebView2. Use an x64 Visual Studio developer terminal so the MSVC linker and
SDK libraries are available. On this machine, initialize PowerShell as follows:

```powershell
Import-Module 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\Microsoft.VisualStudio.DevShell.dll'
Enter-VsDevShell -VsInstallPath 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools' -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64'
$env:Path = 'C:\Users\Karikari\.cargo\bin;' + $env:Path
Set-Location 'C:\Users\Karikari\OneDrive\Documents\Projects\Prompt-Kitcha'
```

Then run:

```powershell
npm ci
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri -- build --bundles nsis --ci
```

Tests cover grounded request/indentation preservation, scope safeguards, malformed
model output, escaped Unicode streaming, packet-split Ollama UTF-8, cancellation
cleanup, editable-selection checks, and the clipboard fallback policy. A
render-only benchmark is available separately:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml benchmark_grounded_rendering --release -- --ignored --nocapture
```

Measured optimized renderer results for 1,000 renders per case:

| Source | Characters | p50 | p95 |
| --- | ---: | ---: | ---: |
| Short request | 38 | 0.3 microseconds | 0.5 microseconds |
| ASCII request | 7,560 | 0.6 microseconds | 1.3 microseconds |
| Unicode request | 7,200 | 22.4 microseconds | 44.5 microseconds |

These cover rendering only: not capture, intent classification, IPC, UI,
replacement, or model inference. Do not compare them directly with another
application's end-to-end timings.

For the edit/Enter acceptance gate, enable Auto-replace and use Instant grounded
mode to avoid API calls. In a new unsaved Notepad document, type
`BEFORE | make a short study plan | AFTER`. Select only the middle request,
press `Ctrl+Shift+Space`, replace the draft wording with `Edited study prompt`,
then press Enter. The required result is
`BEFORE | Edited study prompt | AFTER`, with no extra newline or submission.
Repeat with another prompt to verify focus resets. Check Shift+Enter in the draft,
Escape, and changing the original selection while the popup is open. A changed
field/range/document must refuse replacement without discarding the draft.
Repeat the main test in the VS Code editor and a browser textarea. Record actual
results before treating the workflow as end-to-end verified.

Manual acceptance checks: Notepad, VS Code editor, browser textarea; selection
with Unicode/code/URLs; no selection; password field; changed selection; clipboard
image; rapid regeneration/cancellation; missing provider; Ollama cold/warm model;
startup after sign-in. Verify no insertion occurs after a failed capture or a
selection change. Elevated target apps may reject input from a normal process.
For the terminal fix, also check OpenCode mouse-owned selection versus Alt-drag,
repeatedly copying the same prompt, copying after hiding the pill, clicking the
pill after a newer copy, and Use copied text after native capture fails. Verify
copied-text mode shows Copy & close and never inserts or submits anything in
OpenCode automatically.

## Reference code

The native-first selection policy adapts
[AI Prompt Optimizer](https://github.com/jiusimeng8-cmd/AI_Prompt_Optimizer).
The source-grounded four-section renderer follows
[Flowkey / Fastflow](https://github.com/agr77one/Fastflow), without its clause
splitting/truncation or a redundant model draft call in instant mode. Exact commits,
source paths, and MIT notices are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
