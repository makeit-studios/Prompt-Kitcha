import { useEffect, useRef, useState } from "react";
import {
  Copy,
  Check,
  RefreshCw,
  X,
  ArrowRightLeft,
  Settings2,
  SendHorizonal,
  Sparkles,
  AlertCircle,
  Loader2,
  ChevronDown,
  MessageSquare,
  Image,
  Lightbulb,
  Wand2,
} from "lucide-react";
import clsx from "clsx";
import { IntentBadge } from "../components/IntentBadge";
import {
  ipc,
  onOptimize,
  onSelectionCaptured,
  onClipboardCaptured,
  onCaptureError,
  type Intent,
  type OptimizeEvent,
} from "../lib/ipc";
import {
  promptTechniques,
  generatePromptWithTechnique,
  getTechniqueById,
  type PromptTechnique,
} from "../lib/promptTechniques";

// Function to suggest best technique based on input
function suggestTechnique(input: string): { technique: PromptTechnique; reason: string } {
  const lower = input.toLowerCase();

  // Check for specific patterns
  if (lower.includes("step") || lower.includes("how to") || lower.includes("process") || lower.includes("explain")) {
    return {
      technique: getTechniqueById("reasoning")!,
      reason: "Step-by-step reasoning works best for explanations"
    };
  }

  if (lower.includes("code") || lower.includes("function") || lower.includes("program")) {
    return {
      technique: getTechniqueById("care")!,
      reason: "CARE framework helps with code examples"
    };
  }

  if (lower.includes("email") || lower.includes("write") || lower.includes("draft")) {
    return {
      technique: getTechniqueById("create")!,
      reason: "CREATE helps craft well-structured writing"
    };
  }

  if (lower.includes("problem") || lower.includes("fix") || lower.includes("issue") || lower.includes("solve")) {
    return {
      technique: getTechniqueById("pain")!,
      reason: "PAIN framework is ideal for problem-solving"
    };
  }

  if (lower.includes("analyze") || lower.includes("compare") || lower.includes("evaluate")) {
    return {
      technique: getTechniqueById("roses")!,
      reason: "ROSES provides structured analysis"
    };
  }

  if (lower.includes("plan") || lower.includes("strategy") || lower.includes("approach")) {
    return {
      technique: getTechniqueById("coast")!,
      reason: "COAST helps with strategic planning"
    };
  }

  if (lower.includes("role") || lower.includes("act as") || lower.includes("expert")) {
    return {
      technique: getTechniqueById("race")!,
      reason: "RACE defines roles clearly"
    };
  }

  // Default to standard
  return {
    technique: getTechniqueById("standard")!,
    reason: "Standard works well for general prompts"
  };
}

type Status = "idle" | "capturing" | "streaming" | "done" | "error" | "cancelled";

export default function Popup() {
  const [source, setSource] = useState("");
  const [intent, setIntent] = useState<Intent | "detecting">("detecting");
  const [output, setOutput] = useState("");
  const [status, setStatus] = useState<Status>("idle");
  const [readyVersion, setReadyVersion] = useState(0);
  const [manualPrompt, setManualPrompt] = useState("");
  const [inputLimit, setInputLimit] = useState(10_000);
  const [editorMode, setEditorMode] = useState(false);
  const editorSessionId = useRef<number | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const activeJob = useRef(0);
  const nextJob = useRef(0);
  const captureId = useRef<number | null>(null);
  const revision = useRef(0);
  const replacing = useRef(false);
  const [applying, setApplying] = useState(false);
  const [autoReplace, setAutoReplace] = useState(true);
  const [timing, setTiming] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [feedback, setFeedback] = useState("");
  const [selectedTechnique, setSelectedTechnique] = useState<string>("standard");
  const [showTechniqueDropdown, setShowTechniqueDropdown] = useState(false);
  const [suggestedMethods, setSuggestedMethods] = useState<string[]>([]);
  const [comments, setComments] = useState<string>("");
  const [showComments, setShowComments] = useState(false);
  const [clipboardFallback, setClipboardFallback] = useState<string | null>(null);
  const [canReplace, setCanReplace] = useState(false);
  const [suggestedTechnique, setSuggestedTechnique] = useState<{ technique: PromptTechnique; reason: string } | null>(null);
  const lastOutputRef = useRef<string>("");
  const outRef = useRef<HTMLDivElement>(null);
  const draftRef = useRef<HTMLTextAreaElement>(null);
  const manualRef = useRef<HTMLTextAreaElement>(null);
  const techniqueDropdownRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const unlistens: Array<Promise<() => void>> = [];
    unlistens.push(
      onSelectionCaptured((capture) => {
        if (editorSessionId.current !== null) return;
        startCaptured(capture.text, capture.replaceable ? capture.capture_id : null);
      })
    );
    unlistens.push(onClipboardCaptured((text) => {
      if (editorSessionId.current === null) startCaptured(text, null);
    }));
    unlistens.push(
      onOptimize((ev: OptimizeEvent) => {
        if (ev.job_id !== activeJob.current) return;
        if (ev.kind === "intent" && ev.intent) setIntent(ev.intent as Intent);
        else if (ev.kind === "delta" && ev.prompt_delta) {
          setOutput((p) => p + ev.prompt_delta);
          setStatus("streaming");
        } else if (ev.kind === "done") {
          activeJob.current = 0;
          if (!ev.full_prompt?.trim()) {
            setErr("The enhancer returned an empty prompt. Try again.");
            setStatus("error");
            return;
          }
          setOutput(ev.full_prompt);
          lastOutputRef.current = ev.full_prompt;
          setReadyVersion((version) => version + 1);
          if (ev.intent) setIntent(ev.intent as Intent);
          setTiming(`${ev.mode === "grounded" ? "Instant" : "AI"} · ${ev.elapsed_ms} ms`);
          setStatus("done");
        } else if (ev.kind === "error") {
          activeJob.current = 0;
          setErr(ev.error ?? "unknown error");
          setStatus("error");
        } else if (ev.kind === "cancelled") {
          activeJob.current = 0;
          setStatus("cancelled");
        }
      })
    );
    unlistens.push(onCaptureError((error) => {
      if (editorSessionId.current !== null) return;
      if (activeJob.current) ipc.cancel(activeJob.current).catch(() => {});
      activeJob.current = 0;
      revision.current += 1;
      captureId.current = null;
      lastOutputRef.current = "";
      setFeedback("");
      setSuggestedMethods([]);
      setSuggestedTechnique(null);
      setOutput("");
      setSource("");
      setCanReplace(false);
      setClipboardFallback(null);
      setErr(error);
      setStatus("error");
    }));
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !e.isComposing && e.keyCode !== 229 && !replacing.current) {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      unlistens.forEach((u) => u.then((fn) => fn()));
      window.removeEventListener("keydown", onKey);
    };
  }, []);

  useEffect(() => {
    if (isBusy && outRef.current) outRef.current.scrollTop = outRef.current.scrollHeight;
  }, [output, status]);

  useEffect(() => {
    if (status === "done") {
      draftRef.current?.focus();
      draftRef.current?.setSelectionRange(0, 0);
    }
  }, [status, readyVersion]);

  useEffect(() => {
    ipc.getConfig().then((cfg) => setInputLimit(cfg.max_input_chars)).catch(() => {});
  }, []);

  useEffect(() => {
    let disposed = false;
    ipc.getEditorSession().then((session) => {
      if (disposed || !session) return;
      editorSessionId.current = session.session_id;
      setEditorMode(true);
      if (session.text.trim()) startCaptured(session.text, null);
    }).catch((error) => {
      if (!disposed) setErr(String(error));
    });
    return () => { disposed = true; };
  }, []);

  useEffect(() => {
    if (!source) manualRef.current?.focus();
  }, [source, status]);

  function startCaptured(text: string, id: number | null) {
    const currentRevision = ++revision.current;
    captureId.current = id;
    lastOutputRef.current = "";
    setFeedback("");
    setCopied(false);
    setSelectedTechnique("standard");
    setSuggestedMethods([]);
    setComments("");
    setShowComments(false);
    setShowTechniqueDropdown(false);
    setSource(text);
    setManualPrompt(text);
    setCanReplace(id !== null);
    setClipboardFallback(editorSessionId.current !== null ? "OpenCode editor draft: Enter returns your edited prompt without submitting it. Esc keeps the original." : id !== null ? null : "No validated editable selection: edit the result, then press Enter to Copy & close. Paste manually into your prompt input. Automatic replacement is disabled.");
    ipc.getConfig().then((cfg) => {
      if (revision.current === currentRevision) {
        setAutoReplace(cfg.auto_replace);
        setInputLimit(cfg.max_input_chars);
      }
    }).catch(() => {});
    setSuggestedTechnique(suggestTechnique(text));
    runOptimize(text);
  }

  async function useCopiedText() {
    try {
      await ipc.showPopupAtCursor();
      const text = await ipc.readClipboard();
      if (!text.trim()) throw new Error("Copy your prompt as text first, then try again.");
      startCaptured(text, null);
    } catch (error) {
      setErr(String(error));
      setStatus("error");
    }
  }

  function editSource() {
    if (isBusy || replacing.current) return;
    revision.current += 1;
    captureId.current = null;
    activeJob.current = 0;
    setManualPrompt(source);
    setSource("");
    setOutput("");
    lastOutputRef.current = "";
    setFeedback("");
    setCanReplace(false);
    setClipboardFallback(null);
    setSuggestedTechnique(null);
    setSuggestedMethods([]);
    setErr(null);
    setStatus("idle");
    ipc.getConfig().then((cfg) => setInputLimit(cfg.max_input_chars)).catch(() => {});
  }

  async function pastePrompt() {
    const currentRevision = revision.current;
    try {
      const text = await ipc.readClipboard();
      if (revision.current !== currentRevision || source) return;
      if (!text.trim()) throw new Error("Copy your prompt as text first, then try again.");
      setManualPrompt(text);
      setErr(null);
      manualRef.current?.focus();
    } catch (error) {
      if (revision.current === currentRevision) setErr(String(error));
    }
  }

  function enhanceManualPrompt() {
    if (!manualPrompt.trim() || isBusy || applying) return;
    startCaptured(manualPrompt, null);
  }

  async function runOptimize(text: string, opts?: { feedback?: string; previous?: string }) {
    const priorId = activeJob.current;
    const requestedId = Math.max(Date.now(), nextJob.current + 1);
    nextJob.current = requestedId;
    activeJob.current = requestedId;
    setOutput("");
    setTiming(null);
    setErr(null);
    setIntent("detecting");
    setStatus("capturing");
    try {
      if (priorId) await ipc.cancel(priorId).catch(() => {});
      if (activeJob.current !== requestedId) return;
      const id = await ipc.optimize({
        text,
        job_id: requestedId,
        feedback: opts?.feedback ?? null,
        previous_output: opts?.previous ?? null,
      });
      if (activeJob.current !== requestedId) await ipc.cancel(id).catch(() => {});
    } catch (e: any) {
      if (activeJob.current !== requestedId) return;
      activeJob.current = 0;
      setErr(String(e));
      setStatus("error");
    }
  }

  async function refine() {
    if (!source || !feedback.trim() || isBusy || applying) return;
    const prev = lastOutputRef.current;
    const instructions = feedback;
    await runOptimize(source, { feedback: instructions, previous: prev });
  }

  async function accept() {
    if (!output.trim() || status !== "done" || replacing.current) return;
    const currentRevision = revision.current;
    const originalCapture = captureId.current;
    const editedDraft = output;
    replacing.current = true;
    setApplying(true);
    setErr(null);
    try {
      if (editorSessionId.current !== null) {
        await ipc.saveEditorPrompt(editedDraft, editorSessionId.current);
        if (revision.current === currentRevision) await ipc.quit();
        return;
      }
      const cfg = await ipc.getConfig();
      if (revision.current !== currentRevision) return;
      setAutoReplace(cfg.auto_replace);
      if (cfg.auto_replace && originalCapture !== null) {
        await ipc.replaceSelection(editedDraft, originalCapture);
      } else {
        await ipc.writeClipboard(editedDraft);
      }
      if (revision.current === currentRevision) await ipc.hideWindow("popup");
    } catch (e) {
      if (revision.current === currentRevision) {
        setErr(String(e));
        await ipc.showWindow("popup").catch(() => {});
      }
    } finally {
      replacing.current = false;
      setApplying(false);
    }
  }

  async function copy() {
    if (!output.trim() || status !== "done" || replacing.current) return;
    try {
      await ipc.writeClipboard(output);
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    } catch (error) {
      setErr(String(error));
    }
  }

  async function regenerate() {
    if (!source || isBusy || applying) return;
    await runOptimize(source);
  }

  async function close() {
    if (replacing.current) return;
    revision.current += 1;
    captureId.current = null;
    const current = activeJob.current;
    activeJob.current = 0;
    if (current) await ipc.cancel(current).catch(() => {});
    if (editorSessionId.current !== null) {
      await ipc.quit().catch((error) => setErr(String(error)));
      return;
    }
    await ipc.hideWindow("popup").catch((error) => setErr(String(error)));
  }

  const isBusy = status === "streaming" || status === "capturing";
  const willReplace = !editorMode && canReplace && autoReplace;

  const currentTechnique = promptTechniques.find((t) => t.id === selectedTechnique);

  const handleTechniqueSelect = (techniqueId: string) => {
    setSelectedTechnique(techniqueId);
    setShowTechniqueDropdown(false);
    setSuggestedTechnique(null); // Clear suggestion when user manually selects
    if (source && !isBusy && !applying) {
      const result = generatePromptWithTechnique(techniqueId, source);
      setSuggestedMethods(result.suggestedMethods);
      // Re-run optimization with the new technique
      runOptimize(source);
    }
  };

  const applySuggestedTechnique = () => {
    if (suggestedTechnique) {
      handleTechniqueSelect(suggestedTechnique.technique.id);
    }
  };

  const applyMethod = (method: string) => {
    if (status !== "done" || applying) return;
    const methodText = `\n${method}: [Add details here]`;
    const draft = output + methodText;
    lastOutputRef.current = draft;
    setOutput(draft);
    draftRef.current?.focus();
  };

  // Close dropdown when clicking outside
  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (
        techniqueDropdownRef.current &&
        !techniqueDropdownRef.current.contains(event.target as Node)
      ) {
        setShowTechniqueDropdown(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  return (
    <div className="h-screen w-screen p-2 animate-slide-up">
      <div
        data-tauri-drag-region
        className="rounded-2xl glass shadow-float ring-1 ring-black/10 dark:ring-white/10
        flex flex-col h-full overflow-hidden"
      >
        <div
          data-tauri-drag-region
          className="flex items-center gap-2 px-3 py-2 border-b border-black/5 dark:border-white/10
          bg-gradient-to-r from-white/40 to-white/0 dark:from-white/5 dark:to-transparent"
        >
          <IntentBadge intent={intent} />
          <span className="text-[11px] font-medium text-zinc-700 dark:text-zinc-200 truncate flex items-center gap-1">
            {isBusy && <Loader2 size={11} className="animate-spin" />}
            {applying ? "Applying..." : statusLabel(status, source.length, willReplace, editorMode)}
            {status === "done" && timing && <span className="ml-1 text-zinc-500">{timing}</span>}
          </span>
          <div className="flex-1" />

          {/* Technique Dropdown */}
          <div className="relative" ref={techniqueDropdownRef}>
            <button
              onClick={() => setShowTechniqueDropdown(!showTechniqueDropdown)}
              disabled={isBusy || applying}
              className="btn-ghost text-xs flex items-center gap-1"
              title="Select prompt technique"
            >
              {currentTechnique?.name || "Standard"}
              <ChevronDown size={12} />
            </button>
            {showTechniqueDropdown && (
              <div className="absolute right-0 top-full mt-1 w-56 max-h-64 overflow-y-auto
              bg-white dark:bg-zinc-800 rounded-lg shadow-lg ring-1 ring-black/10 dark:ring-white/10
              z-50 py-1">
                {promptTechniques.map((technique) => (
                  <button
                    key={technique.id}
                    onClick={() => handleTechniqueSelect(technique.id)}
                    className={clsx(
                      "w-full text-left px-3 py-2 text-xs hover:bg-zinc-100 dark:hover:bg-zinc-700 transition-colors",
                      selectedTechnique === technique.id && "bg-brand-50 dark:bg-brand-900/20 text-brand-600 dark:text-brand-400"
                    )}
                  >
                    <div className="font-medium">{technique.name}</div>
                    <div className="text-zinc-500 dark:text-zinc-400 text-[10px] truncate">
                      {technique.description}
                    </div>
                  </button>
                ))}
              </div>
            )}
          </div>

          <button onClick={() => ipc.showSettings()} className="btn-icon" title="Settings">
            <Settings2 size={14} />
          </button>
          <button onClick={close} disabled={applying} className="btn-icon" title="Close (Esc)">
            <X size={14} />
          </button>
        </div>

        {/* Suggestion Bar */}
        {suggestedTechnique && suggestedTechnique.technique.id !== selectedTechnique && (
          <div className="px-3 py-2 border-b border-black/5 dark:border-white/10 bg-gradient-to-r from-amber-50/80 to-orange-50/60 dark:from-amber-900/20 dark:to-orange-900/10">
            <div className="flex items-center gap-2">
              <Lightbulb size={14} className="text-amber-600 dark:text-amber-400 shrink-0" />
              <span className="text-xs text-zinc-700 dark:text-zinc-300 flex-1">
                <span className="font-medium">Suggestion:</span> {suggestedTechnique.reason}
              </span>
              <button
                onClick={applySuggestedTechnique}
                className="px-2.5 py-1 rounded-md text-[11px] font-medium bg-amber-100 dark:bg-amber-900/30 text-amber-800 dark:text-amber-300 hover:bg-amber-200 dark:hover:bg-amber-900/50 transition-colors flex items-center gap-1"
              >
                <Wand2 size={10} />
                Use {suggestedTechnique.technique.name}
              </button>
            </div>
          </div>
        )}

        <div
          ref={outRef}
          className="flex-1 min-h-0 overflow-auto px-4 py-3 text-[13px] leading-relaxed whitespace-pre-wrap
          text-zinc-800 dark:text-zinc-100 selection:bg-brand-500/25"
        >
          {err && (
            <div role="alert" className="flex gap-2 p-3 rounded-lg bg-rose-50 dark:bg-rose-500/10 ring-1 ring-rose-200 dark:ring-rose-500/20 text-rose-700 dark:text-rose-300">
              <AlertCircle size={16} className="shrink-0 mt-0.5" />
              <div>
                <div className="font-semibold mb-0.5 text-[13px]">Something went wrong</div>
                <div className="whitespace-pre-wrap text-xs opacity-90">{err}</div>
                {!source && (
                  <button onClick={useCopiedText} className="btn-ghost mt-2" title="Enhance the text you manually copied; never sends Ctrl+C to a terminal">
                    <Copy size={12} /> Use copied text
                  </button>
                )}
              </div>
            </div>
          )}
          {!source ? (
            <div className="flex flex-col h-full min-h-24 gap-2">
              <textarea
                ref={manualRef}
                aria-label="Prompt to enhance"
                aria-describedby="source-keyboard-help"
                value={manualPrompt}
                onChange={(event) => setManualPrompt(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key !== "Enter" || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
                  if (event.shiftKey || event.ctrlKey || event.altKey || event.metaKey) return;
                  event.preventDefault();
                  if (!event.repeat) enhanceManualPrompt();
                }}
                placeholder="Paste your complete prompt here, including multiple lines."
                spellCheck={false}
                className="w-full flex-1 min-h-16 resize-none bg-transparent outline-none text-[13px] leading-relaxed"
              />
              <div id="source-keyboard-help" className="text-[10px] text-zinc-500">
                {manualPrompt ? manualPrompt.split(/\r\n|\r|\n/).length : 0} lines · {Array.from(manualPrompt).length.toLocaleString()} characters · limit {inputLimit?.toLocaleString()} · Enter: enhance · Shift+Enter: new line
              </div>
              <div className="flex gap-2">
                <button onClick={pastePrompt} className="btn-ghost">Paste from clipboard</button>
                <button onClick={enhanceManualPrompt} disabled={!manualPrompt.trim() || isBusy || applying} className="btn-primary">Enhance</button>
              </div>
            </div>
          ) : status === "done" ? (
            <textarea
              ref={draftRef}
              aria-label="Enhanced prompt"
              aria-describedby="draft-keyboard-help"
              value={output}
              readOnly={applying}
              onChange={(event) => {
                setOutput(event.target.value);
                lastOutputRef.current = event.target.value;
                setCopied(false);
                setErr(null);
              }}
              onKeyDown={(event) => {
                if (event.key !== "Enter" || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
                if (event.shiftKey || event.ctrlKey || event.altKey || event.metaKey) return;
                event.preventDefault();
                if (!event.repeat) accept();
              }}
              spellCheck={false}
              className="w-full h-full min-h-24 resize-none bg-transparent outline-none text-[13px] leading-relaxed text-zinc-800 dark:text-zinc-100 selection:bg-brand-500/25"
            />
          ) : !err && output ? (
            <>
              {output}
              {status === "streaming" && (
                <span className="inline-block w-[3px] h-[14px] bg-brand-500 animate-pulse-soft ml-0.5 align-middle rounded-sm" />
              )}
            </>
          ) : !err && (
            <div className="flex items-center gap-2 text-zinc-400">
              <Sparkles size={14} className="animate-pulse-soft" />
              <span className="text-xs font-medium">Optimizing your prompt…</span>
            </div>
          )}
        </div>

        {/* Suggested Methods */}
        {status === "done" && !err && suggestedMethods.length > 0 && (
          <div className="px-3 py-2 border-t border-black/5 dark:border-white/10 bg-zinc-50/30 dark:bg-zinc-900/30">
            <div className="text-[10px] font-medium text-zinc-500 dark:text-zinc-400 mb-1.5">
              Suggested methods:
            </div>
            <div className="flex flex-wrap gap-1.5">
              {suggestedMethods.map((method) => (
                <button
                  key={method}
                  onClick={() => applyMethod(method)}
                  className="px-2 py-0.5 rounded-full text-[10px] bg-brand-100 dark:bg-brand-900/30
                  text-brand-700 dark:text-brand-300 hover:bg-brand-200 dark:hover:bg-brand-900/50
                  transition-colors"
                >
                  {method}
                </button>
              ))}
            </div>
          </div>
        )}

        {/* Clipboard fallback notice (if any) */}
        {clipboardFallback && (
          <div className="px-3 py-2 border-t border-black/5 dark:border-white/10 bg-yellow-50 dark:bg-yellow-900/20">
            <span className="text-xs text-yellow-800 dark:text-yellow-300">{clipboardFallback}</span>
            <button
              onClick={() => setClipboardFallback(null)}
              className="ml-2 text-xs font-medium text-blue-600 dark:text-blue-300 hover:underline"
            >
              Dismiss
            </button>
          </div>
        )}

        {/* Comments Section */}
        {showComments && (
          <div className="px-3 py-2 border-t border-black/5 dark:border-white/10 bg-white/30 dark:bg-white/[0.02]">
            <div className="text-[10px] font-medium text-zinc-500 dark:text-zinc-400 mb-1.5">
              Comments:
            </div>
            <textarea
              value={comments}
              onChange={(e) => setComments(e.target.value)}
              placeholder="Add your notes or comments here..."
              className="w-full h-16 px-2 py-1.5 rounded-lg bg-white/80 dark:bg-zinc-800/70 text-xs
              ring-1 ring-black/10 dark:ring-white/10 outline-none resize-none
              focus:ring-2 focus:ring-brand-500/50 focus:bg-white dark:focus:bg-zinc-800
              placeholder:text-zinc-400 transition text-zinc-900 dark:text-zinc-100"
            />
          </div>
        )}

        {/* Refine Input - Always Visible */}
        {source && <div className="flex shrink-0 items-center gap-2 px-3 py-2 border-t border-black/5 dark:border-white/10 bg-white/30 dark:bg-white/[0.02]">
          <div className="relative flex-1">
            <textarea
              aria-label="Refinement instructions"
              rows={2}
              value={feedback}
              onChange={(e) => setFeedback(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.nativeEvent.isComposing && e.nativeEvent.keyCode !== 229 && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey) {
                  e.preventDefault();
                  if (!e.repeat) refine();
                }
              }}
              placeholder={status === "done" && !err ? "Refine — e.g. shorter, more casual, add a CTA…" : "Add instructions to improve the prompt…"}
              disabled={!source || isBusy || applying}
              className="w-full resize-none overflow-auto pl-3 pr-9 py-1.5 rounded-lg bg-white/80 dark:bg-zinc-800/70 text-xs
              ring-1 ring-black/10 dark:ring-white/10 outline-none
              focus:ring-2 focus:ring-brand-500/50 focus:bg-white dark:focus:bg-zinc-800
              placeholder:text-zinc-400 transition text-zinc-900 dark:text-zinc-100
              disabled:opacity-50 disabled:cursor-not-allowed"
            />
            <button
              onClick={refine}
              disabled={!feedback.trim() || !source || isBusy || applying}
              title="Refine (Enter)"
              className="absolute right-1 top-1/2 -translate-y-1/2 p-1.5 rounded-md
              text-brand-600 dark:text-brand-400
              hover:bg-brand-500/10 disabled:opacity-30
              disabled:hover:bg-transparent transition-colors"
            >
              <SendHorizonal size={13} />
            </button>
          </div>
        </div>}

        <div className="flex shrink-0 flex-wrap items-center gap-1 px-2 py-2 border-t border-black/5 dark:border-white/10
        bg-zinc-50/60 dark:bg-zinc-950/40">
          <button onClick={regenerate} className="btn-ghost" disabled={!source || isBusy || applying} title="Regenerate">
            <RefreshCw size={12} /> Regenerate
          </button>
          <button onClick={editSource} className="btn-ghost" disabled={!source || isBusy || applying} title={editorMode ? "Edit the original OpenCode draft" : "Edit source as a new copy-only prompt"}>Edit source</button>
          <button onClick={copy} className="btn-ghost" disabled={!output.trim() || status !== "done" || applying} title="Copy">
            {copied ? <Check size={12} className="text-emerald-500" /> : <Copy size={12} />}
            {copied ? "Copied" : "Copy"}
          </button>
          <button
            onClick={() => setShowComments(!showComments)}
            className={clsx("btn-ghost", showComments && "text-brand-600 dark:text-brand-400")}
            title="Toggle comments"
          >
            <MessageSquare size={12} /> Comments
          </button>
          <button
            onClick={() => ipc.showImagePrompt()}
            className="btn-ghost"
            title="Open Image Prompt Maker"
          >
            <Image size={12} /> Image
          </button>
          <div className="flex-1" />
          <button
            onClick={accept}
            disabled={!output.trim() || status !== "done" || applying}
            className="btn-primary"
            title={editorMode ? "Return the edited draft to OpenCode (Enter); does not submit" : willReplace ? "Replace selected text (Enter)" : "Copy the result and close (Enter); paste manually"}
          >
            {applying ? <Loader2 size={12} className="animate-spin" /> : willReplace ? <ArrowRightLeft size={12} strokeWidth={2.5} /> : <Copy size={12} />}
            {applying ? "Applying..." : editorMode ? "Return to OpenCode" : willReplace ? "Replace" : "Copy & close"}
          </button>
        </div>
        <div id="draft-keyboard-help" className="px-3 pb-1 text-[10px] text-zinc-500">
          Enter: {editorMode ? "return draft to OpenCode (not submit)" : willReplace ? "replace selection" : "copy & close"} · Shift+Enter: new line · Esc: cancel
        </div>
      </div>
    </div>
  );
}

function statusLabel(s: Status, n: number, canReplace: boolean, editorMode: boolean): string {
  switch (s) {
    case "idle": return "";
    case "capturing": return `Captured ${n} chars`;
    case "streaming": return "Writing";
    case "done": return editorMode ? "Edit, then Enter to return" : canReplace ? "Edit, then Enter to replace" : "Edit, then Enter to copy";
    case "error": return "Error";
    case "cancelled": return "Cancelled";
  }
}
