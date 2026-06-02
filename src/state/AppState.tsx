import {
  createContext,
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
import { displaySpeaker, speakerMap } from "../types";
import { getBackend } from "../backend";
import { languageName } from "../data/languages";
import { isRtl } from "../i18n";

const DEFAULT_SETTINGS: Settings = {
  appLanguage: "ru",
  defaultLangA: "ru",
  defaultLangB: "en",
  providerMode: "local",
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
  error: string | null;
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
  downloadConversation: (id: string) => void;
}

const AppContext = createContext<AppContextValue | null>(null);

function makeId(): string {
  return Math.random().toString(36).slice(2, 10);
}

export function AppProvider({ children }: { children: ReactNode }) {
  const backend = getBackend();
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [messages, setMessages] = useState<Record<string, Message[]>>({});
  const [activeId, setActiveId] = useState<string | null>(null);
  const [view, setView] = useState<View>("transcript");
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [recording, setRecording] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [summaryOpen, setSummaryOpen] = useState(false);
  // In-flight segments (§10.8): keyed implicitly by pendingId. Cleared when the
  // finished message arrives (by pendingId) or the segment is cancelled.
  const [pending, setPending] = useState<PendingSegment[]>([]);

  useEffect(() => {
    let cancelled = false;
    backend
      .bootstrap()
      .then((data) => {
        if (cancelled) return;
        setConversations(data.conversations);
        setMessages(data.messages);
        if (data.settings) setSettings((prev) => ({ ...prev, ...data.settings }));
        setActiveId(data.conversations[0]?.id ?? null);
      })
      .catch((e) => console.error("bootstrap failed", e));
    return () => {
      cancelled = true;
    };
  }, [backend]);

  // Live transcript messages emitted by the Rust recording pipeline.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    backend
      .onTranscriptMessage((m, pendingId) => {
        setMessages((prev) => {
          const list = prev[m.conversationId] ?? [];
          const i = list.findIndex((x) => x.id === m.id);
          const next = i >= 0 ? list.map((x) => (x.id === m.id ? m : x)) : [...list, m];
          return { ...prev, [m.conversationId]: next };
        });
        // The real row replaces its in-flight placeholder (§10.8).
        if (pendingId !== undefined) {
          setPending((prev) => prev.filter((p) => p.pendingId !== pendingId));
        }
        setConversations((prev) =>
          prev.map((c) =>
            c.id === m.conversationId ? { ...c, updatedAt: m.createdAt } : c
          )
        );
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((e) => console.error("transcript subscription failed", e));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [backend]);

  // Non-fatal recording errors from the Rust pipeline (STT/translation), shown
  // as a dismissible toast.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    backend
      .onRecordingError((message) => setError(message))
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch((e) => console.error("error subscription failed", e));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [backend]);

  // Live placeholders (§10.8). A pause raises a "silence" bar that fills over
  // `hangoverMs`; if the speaker resumes it's cancelled, otherwise it flips to
  // a "processing" shimmer until the real message (or a cancel) arrives.
  useEffect(() => {
    let unlistenSilence: (() => void) | undefined;
    let unlistenPending: (() => void) | undefined;
    let unlistenCancelled: (() => void) | undefined;
    let cancelled = false;
    // Insert or update the placeholder for `pendingId`, restarting the phase
    // clock so the CSS bar animates from the new phase's start.
    const upsert = (next: PendingSegment) =>
      setPending((prev) => {
        const i = prev.findIndex((p) => p.pendingId === next.pendingId);
        if (i < 0) return [...prev, next];
        const copy = prev.slice();
        copy[i] = next;
        return copy;
      });
    backend
      .onSegmentSilence((p) => {
        upsert({
          pendingId: p.pendingId,
          conversationId: p.conversationId,
          phase: "silence",
          hangoverMs: p.hangoverMs,
          since: Date.now(),
        });
      })
      .then((fn) => (cancelled ? fn() : (unlistenSilence = fn)))
      .catch((e) => console.error("segment-silence subscription failed", e));
    backend
      .onSegmentPending((p) => {
        upsert({
          pendingId: p.pendingId,
          conversationId: p.conversationId,
          phase: "processing",
          since: Date.now(),
        });
      })
      .then((fn) => (cancelled ? fn() : (unlistenPending = fn)))
      .catch((e) => console.error("segment-pending subscription failed", e));
    backend
      .onSegmentCancelled((pendingId) => {
        setPending((prev) => prev.filter((p) => p.pendingId !== pendingId));
      })
      .then((fn) => (cancelled ? fn() : (unlistenCancelled = fn)))
      .catch((e) => console.error("segment-cancelled subscription failed", e));
    return () => {
      cancelled = true;
      unlistenSilence?.();
      unlistenPending?.();
      unlistenCancelled?.();
    };
  }, [backend]);

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
      error,
      activeConversation,
      activeMessages,
      activePending,
      toggleRecording: () => {
        if (!activeId) return;
        if (recording) {
          setRecording(false);
          backend.stopRecording().catch((e) =>
            console.error("stopRecording failed", e)
          );
        } else {
          setRecording(true);
          setError(null);
          backend.startRecording(activeId).catch((e) => {
            console.error("startRecording failed", e);
            setError(e instanceof Error ? e.message : String(e));
            setRecording(false);
          });
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
        backend.createConversation(conv).catch((e) =>
          console.error("createConversation failed", e)
        );
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
        backend.saveSettings(next).catch((e) =>
          console.error("saveSettings failed", e)
        );
      },
      setConversationLangs: (langA, langB) => {
        if (!activeId) return;
        const ts = Date.now();
        setConversations((prev) =>
          prev.map((c) =>
            c.id === activeId ? { ...c, langA, langB, updatedAt: ts } : c
          )
        );
        backend.setConversationLangs(activeId, langA, langB, ts).catch((e) =>
          console.error("setConversationLangs failed", e)
        );
      },
      swapLanguages: () => {
        if (!activeId || !activeConversation) return;
        const ts = Date.now();
        const langA = activeConversation.langB;
        const langB = activeConversation.langA;
        setConversations((prev) =>
          prev.map((c) =>
            c.id === activeId ? { ...c, langA, langB, updatedAt: ts } : c
          )
        );
        backend.setConversationLangs(activeId, langA, langB, ts).catch((e) =>
          console.error("swapLanguages failed", e)
        );
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
        setConversations((prev) =>
          prev.map((c) => (c.id === activeId ? { ...c, updatedAt: ts } : c))
        );
        backend.addMessage(msg).catch((e) =>
          console.error("addMessage failed", e)
        );
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
            backend.addMessage(updated).catch((e) =>
              console.error("persist translation failed", e)
            );
          })
          .catch((e) => console.error("translate failed", e));
      },
      renameConversation: (id, title) => {
        const t = title.trim();
        if (!t) return;
        const ts = Date.now();
        setConversations((prev) =>
          prev.map((c) => (c.id === id ? { ...c, title: t, updatedAt: ts } : c))
        );
        backend.renameConversation(id, t, ts).catch((e) =>
          console.error("renameConversation failed", e)
        );
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
        setConversations((prev) =>
          prev.map((c) =>
            c.id === activeId ? { ...c, speakerNames: json, updatedAt: ts } : c
          )
        );
        backend.setSpeakerNames(activeId, json, ts).catch((e) =>
          console.error("setSpeakerNames failed", e)
        );
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
        backend.setMessageSpeaker(messageId, label).catch((e) =>
          console.error("setMessageSpeaker failed", e)
        );
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
        backend.deleteConversation(id).catch((e) =>
          console.error("deleteConversation failed", e)
        );
      },
      downloadConversation: (id) => {
        const conv = conversations.find((c) => c.id === id);
        if (!conv) return;
        const msgs = messages[id] ?? [];
        const lines = [
          `# ${conv.title}`,
          `${languageName(conv.langA)} ↔ ${languageName(conv.langB)}`,
          "",
        ];
        for (const m of msgs) {
          const name = displaySpeaker(conv, m.speaker);
          const who = name ? `**${name}** ` : "";
          const foreign =
            m.detectedLang !== conv.langA && m.detectedLang !== conv.langB;
          const tag = foreign ? `[${languageName(m.detectedLang)}] ` : "";
          lines.push(`${who}${tag}${m.originalText}`);
          if (m.translatedText) lines.push(`> ${m.translatedText}`);
          if (foreign && m.translatedTextB) lines.push(`> ${m.translatedTextB}`);
          lines.push("");
        }
        const blob = new Blob([lines.join("\n")], {
          type: "text/markdown;charset=utf-8",
        });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = `${conv.title}.md`;
        a.click();
        URL.revokeObjectURL(url);
      },
    };
  }, [backend, conversations, messages, activeId, view, settings, recording, error, summaryOpen, pending]);

  return <AppContext.Provider value={value}>{children}</AppContext.Provider>;
}

export function useApp(): AppContextValue {
  const ctx = useContext(AppContext);
  if (!ctx) throw new Error("useApp must be used within AppProvider");
  return ctx;
}
