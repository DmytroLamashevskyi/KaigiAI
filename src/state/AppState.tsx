import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type {
  Conversation,
  Message,
  PendingSegment,
  Settings,
  SetupIssue,
  View,
} from "../types";
import { conversationLangs, speakerMap } from "../types";
import { getBackend } from "../backend";
import { isRtl } from "../i18n";
import { useRecordingEvents } from "./useRecordingEvents";
import {
  conversationCsv,
  conversationMarkdown,
  conversationPrintHtml,
  detectMessageLang,
  downloadFile,
  errMsg,
  logErr,
  makeId,
  migrateSettings,
  printHtmlViaIframe,
  translateAll,
} from "./helpers";

/** Max conversation languages (§10.7). Beyond this the N-way translation fan-out
 *  and column grid stop being legible. */
export const MAX_LANGS = 5;

const DEFAULT_SETTINGS: Settings = {
  appLanguage: "ru",
  defaultLangA: "ru",
  defaultLangB: "en",
  sttMode: "local",
  translationMode: "local",
  startServersOnLaunch: false,
  apiBaseUrl: "",
  apiKey: "",
  sttModel: "whisper-large-v3",
  llmModel: "qwen2.5-7b-instruct",
  localWhisperServerPath: "",
  localLlmServerPath: "",
  localWhisperPath: "",
  localLlmPath: "",
  diarizationModelPath: "",
  segmentationModelPath: "",
  nGpuLayers: 0,
  audioDevice: "",
  audioSource: "mic",
  silenceMs: 3000,
  detectForeignLanguages: false,
  exportDir: "",
  saveAudio: false,
  fontSize: "medium",
  theme: "light",
  onboarded: false,
};

interface AppContextValue {
  conversations: Conversation[];
  messages: Record<string, Message[]>;
  activeId: string | null;
  view: View;
  settings: Settings;
  recording: boolean;
  /** True while the local servers are being started (model load) before a
   *  recording can begin — drives the "Подготовка сервиса" indicator. */
  preparing: boolean;
  error: string | null;
  /** Transient success/info message (e.g. "saved to …"), shown as a toast. */
  notice: string | null;
  dismissNotice: () => void;
  /** Unmet setup requirements for the current mode (empty = ready). */
  setupIssues: SetupIssue[];
  recheckSetup: () => void;
  /** First-run setup wizard visibility + controls. */
  wizardOpen: boolean;
  openWizard: () => void;
  closeWizard: () => void;
  /** Mark onboarding done and close the wizard. */
  finishOnboarding: () => void;
  activeConversation: Conversation | null;
  activeMessages: Message[];
  /** In-flight segments for the active conversation, shown as live-timer
   *  placeholders below the transcript (§10.8). */
  activePending: PendingSegment[];
  /** Message ids whose translations are currently being fetched, so the grid can
   *  show a "translating…" placeholder instead of an empty-cell em-dash (§10.7). */
  translatingIds: Record<string, boolean>;
  toggleRecording: () => void;
  dismissError: () => void;
  selectConversation: (id: string) => void;
  newConversation: () => void;
  openSettings: () => void;
  closeSettings: () => void;
  summaryOpen: boolean;
  openSummary: () => void;
  closeSummary: () => void;
  summarize: () => Promise<string>;
  updateSettings: (patch: Partial<Settings>) => void;
  setConversationLangs: (langA: string, langB: string) => void;
  swapLanguages: () => void;
  /** Replace the active conversation's full ordered language list (§10.7). Adding
   *  a language only affects *new* messages; existing rows aren't back-translated. */
  setLanguages: (langs: string[]) => void;
  addTextMessage: (text: string) => void;
  renameConversation: (id: string, title: string) => void;
  renameSpeaker: (label: string, name: string) => void;
  /** Reassign a single message to a different speaker label (manual diarization
   *  fix). `label` null clears the attribution. */
  reassignSpeaker: (messageId: string, label: string | null) => void;
  /** Flip a message to the other pair language (fixes a mis-detected side, e.g.
   *  typed text in a same-script pair) and re-translate. 2-language view only. */
  reassignMessageLang: (messageId: string) => void;
  /** Set which conversation language a message was actually spoken in (N-language
   *  manual fix, §10.7) and re-translate the original into every other language. */
  setMessageLang: (messageId: string, newLang: string) => void;
  deleteConversation: (id: string) => void;
  /** Export modal: conversation id being exported, or null when closed. */
  exportId: string | null;
  openExport: (id: string) => void;
  closeExport: () => void;
  /** Save the conversation as a Markdown file. */
  exportMarkdown: (id: string) => void;
  /** Save the conversation as CSV (one row per message, a column per language). */
  exportCsv: (id: string) => void;
  /** Open a print-ready view and trigger the system print dialog ("Save as
   *  PDF") — keeps Japanese/Cyrillic glyphs correct via system fonts. */
  exportPdf: (id: string) => void;
  /** Write a ZIP (transcript + audio clips) to the configured export folder. */
  exportZip: (id: string) => void;
  /** Generate a conversation title with the LLM and apply it. */
  autoTitle: () => Promise<void>;
}

const AppContext = createContext<AppContextValue | null>(null);

export function AppProvider({ children }: { children: ReactNode }) {
  const backend = getBackend();
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [messages, setMessages] = useState<Record<string, Message[]>>({});
  const [activeId, setActiveId] = useState<string | null>(null);
  const [view, setView] = useState<View>("transcript");
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [recording, setRecording] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [summaryOpen, setSummaryOpen] = useState(false);
  const [exportId, setExportId] = useState<string | null>(null);
  const [setupIssues, setSetupIssues] = useState<SetupIssue[]>([]);
  const [wizardOpen, setWizardOpen] = useState(false);
  // In-flight segments (§10.8): keyed implicitly by pendingId. Cleared when the
  // finished message arrives (by pendingId) or the segment is cancelled.
  const [pending, setPending] = useState<PendingSegment[]>([]);
  // Messages with translations in flight (typed input / language reassignment),
  // so the N-column grid distinguishes "translating…" from "no translation".
  const [translatingIds, setTranslatingIds] = useState<Record<string, boolean>>({});
  const markTranslating = useCallback(
    (id: string, on: boolean) =>
      setTranslatingIds((prev) => {
        if (on) return { ...prev, [id]: true };
        if (!prev[id]) return prev;
        const next = { ...prev };
        delete next[id];
        return next;
      }),
    []
  );

  // Merge a patch into one conversation by id (dedupes the map-and-replace
  // pattern used by rename/lang/speaker updates).
  const patchConversation = useCallback(
    (id: string, patch: Partial<Conversation>) =>
      setConversations((prev) =>
        prev.map((c) => (c.id === id ? { ...c, ...patch } : c))
      ),
    []
  );

  // Wholesale-replace one message by id in its conversation's list (the
  // optimistic-then-final update used by typed input and language reassignment).
  const replaceMessage = useCallback(
    (msg: Message) =>
      setMessages((prev) => ({
        ...prev,
        [msg.conversationId]: (prev[msg.conversationId] ?? []).map((x) =>
          x.id === msg.id ? msg : x
        ),
      })),
    []
  );

  // Re-run the readiness check (after settings change, or wizard steps).
  const recheckSetup = useCallback(() => {
    backend.checkSetup().then(setSetupIssues).catch(logErr("checkSetup failed"));
  }, [backend]);

  // Debounced variant for settings edits: every keystroke in a path/key field
  // calls updateSettings, and running the file-stat readiness check on each one
  // would thrash the filesystem (esp. network paths). Coalesce to one check.
  const recheckTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const recheckSetupDebounced = useCallback(() => {
    if (recheckTimer.current) clearTimeout(recheckTimer.current);
    recheckTimer.current = setTimeout(recheckSetup, 400);
  }, [recheckSetup]);

  // Always-current recording flag for event handlers (avoids stale closures and
  // lets late events be ignored once recording has stopped).
  const recordingRef = useRef(false);
  recordingRef.current = recording;

  // Always-current settings for async handlers (re-assigned every render): lets
  // the bootstrap checkSetup callback merge into the settings as they are at
  // resolution time without putting side effects inside a setSettings updater
  // (updaters must be pure — StrictMode double-invokes them in dev).
  const settingsRef = useRef(settings);
  settingsRef.current = settings;

  useEffect(() => {
    let cancelled = false;
    backend
      .bootstrap()
      .then((data) => {
        if (cancelled) return;
        setConversations(data.conversations);
        setMessages(data.messages);
        const merged: Settings = {
          ...DEFAULT_SETTINGS,
          ...(data.settings ? migrateSettings(data.settings) : {}),
        };
        setSettings(merged);
        setActiveId(data.conversations[0]?.id ?? null);
        // Optionally pre-start the local servers so the first recording is
        // instant (the multi-second model load happens now instead).
        const needsLocal =
          merged.sttMode === "local" || merged.translationMode === "local";
        if (merged.startServersOnLaunch && needsLocal) {
          setPreparing(true);
          backend
            .warmupServers()
            .catch((e) => console.error("warmupServers failed", e))
            .finally(() => {
              if (!cancelled) setPreparing(false);
            });
        }
        // Readiness: surface missing config and open the first-run wizard when
        // setup is incomplete and the user hasn't onboarded yet. A complete
        // setup is silently marked onboarded so the wizard never nags.
        backend
          .checkSetup()
          .then((issues) => {
            if (cancelled) return;
            setSetupIssues(issues);
            if (issues.length === 0) {
              if (!merged.onboarded) {
                // Merge into CURRENT settings via the ref (not the stale
                // `merged` snapshot) so edits made while checkSetup was in
                // flight aren't clobbered — without smuggling the save into a
                // setSettings updater (which must stay pure).
                const next = { ...settingsRef.current, onboarded: true };
                setSettings(next);
                backend.saveSettings(next).catch(logErr("saveSettings failed"));
              }
            } else if (!merged.onboarded) {
              setWizardOpen(true);
            }
          })
          .catch(logErr("checkSetup failed"));
      })
      .catch(logErr("bootstrap failed"));
    return () => {
      cancelled = true;
    };
  }, [backend]);

  // Live recording events (transcript rows, errors, §10.8 placeholders).
  useRecordingEvents(backend, {
    setMessages,
    setPending,
    setConversations,
    setError,
    recordingRef,
  });

  // Stop leaves no segment in flight; clear any stragglers so a stale
  // placeholder never lingers after recording ends.
  useEffect(() => {
    if (!recording) setPending([]);
  }, [recording]);

  // Safety cap on "preparing": the backend fails a stuck server start in ~45 s
  // per stage, but if startRecording never resolves at all, don't spin forever.
  useEffect(() => {
    if (!preparing) return;
    const id = setTimeout(() => {
      setPreparing(false);
      setError(
        "Сервис не запустился вовремя. Проверьте пути к серверам/моделям в настройках."
      );
    }, 120_000);
    return () => clearTimeout(id);
  }, [preparing]);

  // Safety net: if a placeholder never resolves (e.g. a transcript-message /
  // segment-cancelled event was lost, or a server hung past its timeout), drop
  // it after a while so the bar can't spin forever. The backend's own request
  // timeout (~90 s) normally clears it first.
  useEffect(() => {
    if (pending.length === 0) return;
    const id = setInterval(() => {
      const cutoff = Date.now() - 120_000;
      setPending((prev) => prev.filter((p) => p.since >= cutoff));
    }, 5000);
    return () => clearInterval(id);
  }, [pending.length]);

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", settings.theme);
  }, [settings.theme]);

  useEffect(() => {
    document.documentElement.lang = settings.appLanguage;
    document.documentElement.dir = isRtl(settings.appLanguage) ? "rtl" : "ltr";
  }, [settings.appLanguage]);

  const value = useMemo<AppContextValue>(() => {
    const activeConversation =
      conversations.find((c) => c.id === activeId) ?? null;
    const activeMessages = activeId ? messages[activeId] ?? [] : [];
    const activePending = activeId
      ? pending.filter((p) => p.conversationId === activeId)
      : [];

    // Set the active conversation's 2-language pair, keeping `langs` in
    // lockstep with the selector (mirrors the DB). Shared by the A/B selects
    // and the ⇄ swap; `ctx` keeps their distinct console-error prefixes.
    const applyLangPair = (langA: string, langB: string, ctx: string) => {
      if (!activeId) return;
      const ts = Date.now();
      patchConversation(activeId, { langA, langB, langs: [langA, langB], updatedAt: ts });
      backend.setConversationLangs(activeId, langA, langB, ts).catch(logErr(ctx));
    };

    return {
      conversations,
      messages,
      activeId,
      view,
      settings,
      recording,
      preparing,
      error,
      notice,
      dismissNotice: () => setNotice(null),
      setupIssues,
      recheckSetup,
      wizardOpen,
      openWizard: () => setWizardOpen(true),
      closeWizard: () => setWizardOpen(false),
      finishOnboarding: () => {
        const next = { ...settings, onboarded: true };
        setSettings(next);
        backend.saveSettings(next).catch(logErr("saveSettings failed"));
        setWizardOpen(false);
        recheckSetup();
      },
      activeConversation,
      activeMessages,
      activePending,
      translatingIds,
      toggleRecording: () => {
        if (!activeId || preparing) return;
        if (recording) {
          setRecording(false);
          backend.stopRecording().catch(logErr("stopRecording failed"));
        } else {
          // startRecording resolves only after the local servers are ready and
          // capture has begun, so we sit in a "preparing" state until then.
          setError(null);
          setPreparing(true);
          backend
            .startRecording(activeId)
            .then(() => setRecording(true))
            .catch((e) => {
              console.error("startRecording failed", e);
              setError(errMsg(e));
            })
            .finally(() => setPreparing(false));
        }
      },
      dismissError: () => setError(null),
      selectConversation: (id) => {
        setActiveId(id);
        setView("transcript");
      },
      newConversation: () => {
        const id = makeId();
        const ts = Date.now();
        const conv: Conversation = {
          id,
          title: "Новый диалог",
          langA: settings.defaultLangA,
          langB: settings.defaultLangB,
          langs: [settings.defaultLangA, settings.defaultLangB],
          createdAt: ts,
          updatedAt: ts,
        };
        setConversations((prev) => [conv, ...prev]);
        setMessages((prev) => ({ ...prev, [id]: [] }));
        setActiveId(id);
        setView("transcript");
        backend.createConversation(conv).catch(logErr("createConversation failed"));
      },
      openSettings: () => setView("settings"),
      closeSettings: () => setView("transcript"),
      summaryOpen,
      openSummary: () => setSummaryOpen(true),
      closeSummary: () => setSummaryOpen(false),
      summarize: () => {
        if (!activeId) return Promise.resolve("");
        return backend.summarize(activeId, settings.appLanguage);
      },
      updateSettings: (patch) => {
        const next = { ...settings, ...patch };
        setSettings(next);
        // Persist immediately (the backend reads settings from the DB when
        // starting a recording), but debounce the file-stat readiness check so
        // per-keystroke path edits don't thrash the filesystem.
        backend.saveSettings(next).catch(logErr("saveSettings failed"));
        recheckSetupDebounced();
      },
      setConversationLangs: (langA, langB) =>
        applyLangPair(langA, langB, "setConversationLangs failed"),
      swapLanguages: () => {
        if (!activeConversation) return;
        applyLangPair(
          activeConversation.langB,
          activeConversation.langA,
          "swapLanguages failed"
        );
      },
      setLanguages: (langs) => {
        if (!activeId) return;
        // Drop blanks/dupes and cap; the first two mirror into langA/langB.
        const cleaned = langs
          .map((l) => l.trim())
          .filter((l, i, a) => l && a.indexOf(l) === i)
          .slice(0, MAX_LANGS);
        if (cleaned.length < 2) return;
        const ts = Date.now();
        patchConversation(activeId, {
          langs: cleaned,
          langA: cleaned[0],
          langB: cleaned[1],
          updatedAt: ts,
        });
        backend
          .setLanguages(activeId, cleaned, ts)
          .catch(logErr("setLanguages failed"));
      },
      addTextMessage: (text) => {
        if (!activeId || !activeConversation || !text.trim()) return;
        const id = makeId();
        const ts = Date.now();
        const trimmed = text.trim();
        const langs = conversationLangs(activeConversation);
        // Detect which language was typed by script, so the text lands in the
        // right column and is translated the right way (e.g. English typed into
        // a RU↔EN chat → EN side, translate into RU — not "treat as RU").
        const detectedLang = detectMessageLang(trimmed, langs);
        const msg: Message = {
          id,
          conversationId: activeId,
          source: "text",
          detectedLang,
          speaker: null,
          originalText: trimmed,
          translatedText: "",
          startMs: 0,
          endMs: 0,
          createdAt: ts,
        };
        setMessages((prev) => ({
          ...prev,
          [activeId]: [...(prev[activeId] ?? []), msg],
        }));
        patchConversation(activeId, { updatedAt: ts });
        markTranslating(id, true);
        // Persist the original immediately (crash-safety), but keep a handle so
        // the final translated write is strictly ordered AFTER it — both calls
        // hit the destructive add_message (DELETE+reINSERT of message_translation)
        // and an out-of-order empty write would silently wipe the translations.
        const basePersist = backend
          .addMessage(msg)
          .catch(logErr("addMessage failed"));
        // Translate into every other conversation language (§10.7); for a
        // 2-language chat this is a single call, for 3+ it fans out and fills the
        // per-language `translations` map the grid reads.
        const targets = langs.filter((l) => l !== detectedLang);
        const translationsP = translateAll(
          backend,
          trimmed,
          detectedLang,
          targets,
          "translate failed"
        );
        Promise.all([basePersist, translationsP]).then(([, translations]) => {
          const updated: Message = {
            ...msg,
            translations,
            // Legacy column: the first other language (keeps the 2-column view
            // and export working for 2-language chats).
            translatedText: targets.length ? translations[targets[0]] ?? "" : "",
          };
          replaceMessage(updated);
          markTranslating(id, false);
          backend.addMessage(updated).catch(logErr("persist translation failed"));
        });
      },
      renameConversation: (id, title) => {
        const t = title.trim();
        if (!t) return;
        const ts = Date.now();
        patchConversation(id, { title: t, updatedAt: ts });
        backend.renameConversation(id, t, ts).catch(logErr("renameConversation failed"));
      },
      renameSpeaker: (label, name) => {
        if (!activeId) return;
        const conv = conversations.find((c) => c.id === activeId);
        if (!conv) return;
        const map = speakerMap(conv);
        const trimmed = name.trim();
        if (trimmed) map[label] = trimmed;
        else delete map[label];
        const json = JSON.stringify(map);
        const ts = Date.now();
        patchConversation(activeId, { speakerNames: json, updatedAt: ts });
        backend.setSpeakerNames(activeId, json, ts).catch(logErr("setSpeakerNames failed"));
      },
      reassignSpeaker: (messageId, label) => {
        if (!activeId) return;
        setMessages((prev) => {
          const list = prev[activeId];
          if (!list) return prev;
          return {
            ...prev,
            [activeId]: list.map((m) =>
              m.id === messageId ? { ...m, speaker: label } : m
            ),
          };
        });
        backend.setMessageSpeaker(messageId, label).catch(logErr("setMessageSpeaker failed"));
      },
      reassignMessageLang: (messageId) => {
        if (!activeId || !activeConversation) return;
        const { langA, langB } = activeConversation;
        const m = (messages[activeId] ?? []).find((x) => x.id === messageId);
        // Only meaningful for a pair-language row (foreign rows have their own
        // two-column layout); flip A<->B and re-translate the original.
        if (!m || (m.detectedLang !== langA && m.detectedLang !== langB)) return;
        const newLang = m.detectedLang === langA ? langB : langA;
        const to = newLang === langA ? langB : langA;
        const flipped = { ...m, detectedLang: newLang, translatedText: "" };
        replaceMessage(flipped); // optimistic: moves to the other column immediately
        backend.addMessage(flipped).catch(logErr("reassign persist failed"));
        backend
          .translate(m.originalText, newLang, to)
          .then((translatedText) => {
            const done = { ...flipped, translatedText };
            replaceMessage(done);
            backend.addMessage(done).catch(logErr("reassign translate persist failed"));
          })
          .catch(logErr("reassign translate failed"));
      },
      setMessageLang: (messageId, newLang) => {
        if (!activeId || !activeConversation) return;
        const langs = conversationLangs(activeConversation);
        const m = (messages[activeId] ?? []).find((x) => x.id === messageId);
        if (!m || m.detectedLang === newLang) return;
        // The utterance is now in `newLang`; clear stale translations and
        // re-translate into every *other* conversation language.
        const targets = langs.filter((l) => l !== newLang);
        const base: Message = {
          ...m,
          detectedLang: newLang,
          translations: {},
          translatedText: "",
          translatedTextB: null,
        };
        replaceMessage(base); // optimistic: original jumps to the new column immediately
        markTranslating(messageId, true);
        // The message already exists in the DB, so we persist EXACTLY ONCE — with
        // the finished `done`. Writing the empty `base` first would race the final
        // write (both DELETE+reINSERT message_translation) and could wipe it.
        translateAll(
          backend,
          m.originalText,
          newLang,
          targets,
          "reassign translate failed"
        ).then((translations) => {
          const done: Message = {
            ...base,
            translations,
            translatedText: targets.length ? translations[targets[0]] ?? "" : "",
          };
          replaceMessage(done);
          markTranslating(messageId, false);
          backend
            .addMessage(done)
            .catch(logErr("reassign lang translate persist failed"));
        });
      },
      deleteConversation: (id) => {
        setConversations((prev) => prev.filter((c) => c.id !== id));
        setMessages((prev) => {
          const next = { ...prev };
          delete next[id];
          return next;
        });
        if (activeId === id) {
          const remaining = conversations.filter((c) => c.id !== id);
          setActiveId(remaining[0]?.id ?? null);
        }
        backend.deleteConversation(id).catch(logErr("deleteConversation failed"));
      },
      exportId,
      openExport: (id) => setExportId(id),
      closeExport: () => setExportId(null),
      exportMarkdown: (id) => {
        const conv = conversations.find((c) => c.id === id);
        if (!conv) return;
        const md = conversationMarkdown(conv, messages[id] ?? []);
        downloadFile(md, `${conv.title}.md`, "text/markdown;charset=utf-8");
        setExportId(null);
      },
      exportCsv: (id) => {
        const conv = conversations.find((c) => c.id === id);
        if (!conv) return;
        const csv = conversationCsv(conv, messages[id] ?? []);
        downloadFile(csv, `${conv.title}.csv`, "text/csv;charset=utf-8");
        setExportId(null);
      },
      exportPdf: (id) => {
        const conv = conversations.find((c) => c.id === id);
        if (!conv) return;
        printHtmlViaIframe(conversationPrintHtml(conv, messages[id] ?? []));
        setExportId(null);
      },
      exportZip: (id) => {
        const conv = conversations.find((c) => c.id === id);
        if (!conv) return;
        const md = conversationMarkdown(conv, messages[id] ?? []);
        backend
          .exportZip(id, conv.title, md, settings.exportDir)
          .then((path) => setNotice(`Сохранено: ${path}`))
          .catch((e) => setError(errMsg(e)));
        setExportId(null);
      },
      autoTitle: async () => {
        if (!activeId) return;
        try {
          const title = await backend.generateTitle(activeId, settings.appLanguage);
          const t = title.trim();
          if (t) {
            patchConversation(activeId, { title: t, updatedAt: Date.now() });
            backend
              .renameConversation(activeId, t, Date.now())
              .catch(logErr("renameConversation failed"));
          }
        } catch (e) {
          setError(errMsg(e));
        }
      },
    };
  }, [backend, patchConversation, replaceMessage, markTranslating, recheckSetup, recheckSetupDebounced, conversations, messages, activeId, view, settings, recording, preparing, error, notice, summaryOpen, exportId, setupIssues, wizardOpen, pending, translatingIds]);

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppContextValue {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp must be used within AppProvider");
  return ctx;
}
