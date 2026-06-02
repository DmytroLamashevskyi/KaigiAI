import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import type {
  Conversation,
  Message,
  PendingSegment,
  Settings,
  View,
} from "../types";
import { speakerMap } from "../types";
import { getBackend } from "../backend";
import { isRtl } from "../i18n";
import { useRecordingEvents } from "./useRecordingEvents";
import {
  conversationMarkdown,
  conversationPrintHtml,
  logErr,
  makeId,
  migrateSettings,
} from "./helpers";

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
  nGpuLayers: 0,
  audioDevice: "",
  audioSource: "mic",
  silenceMs: 3000,
  detectForeignLanguages: false,
  exportDir: "",
  saveAudio: false,
  fontSize: "medium",
  theme: "light",
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
  activeConversation: Conversation | null;
  activeMessages: Message[];
  /** In-flight segments for the active conversation, shown as live-timer
   *  placeholders below the transcript (§10.8). */
  activePending: PendingSegment[];
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
  addTextMessage: (text: string) => void;
  renameConversation: (id: string, title: string) => void;
  renameSpeaker: (label: string, name: string) => void;
  /** Reassign a single message to a different speaker label (manual diarization
   *  fix). `label` null clears the attribution. */
  reassignSpeaker: (messageId: string, label: string | null) => void;
  deleteConversation: (id: string) => void;
  /** Export modal: conversation id being exported, or null when closed. */
  exportId: string | null;
  openExport: (id: string) => void;
  closeExport: () => void;
  /** Save the conversation as a Markdown file. */
  exportMarkdown: (id: string) => void;
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
  // In-flight segments (§10.8): keyed implicitly by pendingId. Cleared when the
  // finished message arrives (by pendingId) or the segment is cancelled.
  const [pending, setPending] = useState<PendingSegment[]>([]);

  // Merge a patch into one conversation by id (dedupes the map-and-replace
  // pattern used by rename/lang/speaker updates).
  const patchConversation = useCallback(
    (id: string, patch: Partial<Conversation>) =>
      setConversations((prev) =>
        prev.map((c) => (c.id === id ? { ...c, ...patch } : c))
      ),
    []
  );

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
      })
      .catch(logErr("bootstrap failed"));
    return () => {
      cancelled = true;
    };
  }, [backend]);

  // Live recording events (transcript rows, errors, §10.8 placeholders).
  useRecordingEvents(backend, { setMessages, setPending, setConversations, setError });

  // Stop leaves no segment in flight; clear any stragglers so a stale
  // placeholder never lingers after recording ends.
  useEffect(() => {
    if (!recording) setPending([]);
  }, [recording]);

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
      activeConversation,
      activeMessages,
      activePending,
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
              setError(e instanceof Error ? e.message : String(e));
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
        backend.saveSettings(next).catch(logErr("saveSettings failed"));
      },
      setConversationLangs: (langA, langB) => {
        if (!activeId) return;
        const ts = Date.now();
        patchConversation(activeId, { langA, langB, updatedAt: ts });
        backend
          .setConversationLangs(activeId, langA, langB, ts)
          .catch(logErr("setConversationLangs failed"));
      },
      swapLanguages: () => {
        if (!activeId || !activeConversation) return;
        const ts = Date.now();
        const langA = activeConversation.langB;
        const langB = activeConversation.langA;
        patchConversation(activeId, { langA, langB, updatedAt: ts });
        backend
          .setConversationLangs(activeId, langA, langB, ts)
          .catch(logErr("swapLanguages failed"));
      },
      addTextMessage: (text) => {
        if (!activeId || !activeConversation || !text.trim()) return;
        const id = makeId();
        const ts = Date.now();
        const msg: Message = {
          id,
          conversationId: activeId,
          source: "text",
          detectedLang: activeConversation.langA,
          speaker: null,
          originalText: text.trim(),
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
        backend.addMessage(msg).catch(logErr("addMessage failed"));
        const convId = activeId;
        backend
          .translate(msg.originalText, activeConversation.langA, activeConversation.langB)
          .then((translatedText) => {
            const updated = { ...msg, translatedText };
            setMessages((prev) => ({
              ...prev,
              [convId]: (prev[convId] ?? []).map((m) =>
                m.id === id ? updated : m
              ),
            }));
            backend.addMessage(updated).catch(logErr("persist translation failed"));
          })
          .catch(logErr("translate failed"));
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
        const blob = new Blob([md], { type: "text/markdown;charset=utf-8" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = `${conv.title}.md`;
        a.click();
        URL.revokeObjectURL(url);
        setExportId(null);
      },
      exportPdf: (id) => {
        const conv = conversations.find((c) => c.id === id);
        if (!conv) return;
        // Print a clean transcript via a hidden iframe → the system print dialog
        // ("Save as PDF"). An iframe works in the Tauri webview (window.open does
        // not), and WebView2's print uses real system fonts so Japanese/Cyrillic
        // render correctly (a bundled-font PDF generator wouldn't).
        const html = conversationPrintHtml(conv, messages[id] ?? []);
        const iframe = document.createElement("iframe");
        iframe.setAttribute("aria-hidden", "true");
        Object.assign(iframe.style, {
          position: "fixed",
          right: "0",
          bottom: "0",
          width: "0",
          height: "0",
          border: "0",
        });
        document.body.appendChild(iframe);
        const cw = iframe.contentWindow;
        if (cw) {
          cw.document.open();
          cw.document.write(html);
          cw.document.close();
          cw.onafterprint = () => setTimeout(() => iframe.remove(), 300);
          setTimeout(() => {
            cw.focus();
            cw.print();
          }, 250);
        }
        setExportId(null);
      },
      exportZip: (id) => {
        const conv = conversations.find((c) => c.id === id);
        if (!conv) return;
        const md = conversationMarkdown(conv, messages[id] ?? []);
        backend
          .exportZip(id, conv.title, md, settings.exportDir)
          .then((path) => setNotice(`Сохранено: ${path}`))
          .catch((e) => setError(e instanceof Error ? e.message : String(e)));
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
          setError(e instanceof Error ? e.message : String(e));
        }
      },
    };
  }, [backend, patchConversation, conversations, messages, activeId, view, settings, recording, preparing, error, notice, summaryOpen, exportId, pending]);

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppContextValue {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp must be used within AppProvider");
  return ctx;
}
