import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Conversation, Message, Settings } from "../types";
import { MOCK_CONVERSATIONS, MOCK_MESSAGES } from "../data/mock";

export interface BootstrapData {
  conversations: Conversation[];
  messages: Record<string, Message[]>;
  settings: Partial<Settings> | null;
}

// The client never needs to know whether persistence is local (Tauri/SQLite)
// or remote (future Axum server) — it only talks to this interface.
// See docs/PROJECT.md §6, §14.
export interface Backend {
  bootstrap(): Promise<BootstrapData>;
  createConversation(c: Conversation): Promise<void>;
  renameConversation(id: string, title: string, updatedAt: number): Promise<void>;
  setConversationLangs(
    id: string,
    langA: string,
    langB: string,
    updatedAt: number
  ): Promise<void>;
  setSpeakerNames(id: string, namesJson: string, updatedAt: number): Promise<void>;
  // Reassign a single message to a different speaker label (manual diarization
  // fix). `label` null clears the attribution. See docs/PROJECT.md §10.9.
  setMessageSpeaker(messageId: string, label: string | null): Promise<void>;
  deleteConversation(id: string): Promise<void>;
  addMessage(m: Message): Promise<void>;
  saveSettings(s: Settings): Promise<void>;
  translate(text: string, from: string, to: string): Promise<string>;
  summarize(conversationId: string, lang: string): Promise<string>;
  startRecording(conversationId: string): Promise<void>;
  stopRecording(): Promise<void>;
  listAudioDevices(): Promise<string[]>;
  // Subscribe to live transcript messages emitted while recording. Returns an
  // unlisten function. No-op outside the desktop shell.
  onTranscriptMessage(cb: (m: Message) => void): Promise<() => void>;
  // Subscribe to non-fatal recording errors (STT/translation failures) surfaced
  // as a toast. Returns an unlisten function. No-op outside the desktop shell.
  onRecordingError(cb: (message: string) => void): Promise<() => void>;
}

// --- Tauri backend (desktop, SQLite via Rust core) ---------------------------

function tauriBackend(): Backend {
  return {
    bootstrap: () => invoke<BootstrapData>("bootstrap"),
    createConversation: (conversation) =>
      invoke<void>("create_conversation", { conversation }),
    renameConversation: (id, title, updatedAt) =>
      invoke<void>("rename_conversation", { id, title, updatedAt }),
    setConversationLangs: (id, langA, langB, updatedAt) =>
      invoke<void>("set_conversation_langs", { id, langA, langB, updatedAt }),
    setSpeakerNames: (id, namesJson, updatedAt) =>
      invoke<void>("set_speaker_names", { id, namesJson, updatedAt }),
    setMessageSpeaker: (messageId, label) =>
      invoke<void>("set_message_speaker", { messageId, label }),
    deleteConversation: (id) => invoke<void>("delete_conversation", { id }),
    addMessage: (message) => invoke<void>("add_message", { message }),
    saveSettings: (settings) => invoke<void>("save_settings", { settings }),
    translate: (text, from, to) =>
      invoke<string>("translate_text", { text, from, to }),
    summarize: (conversationId, lang) =>
      invoke<string>("summarize_conversation", { conversationId, lang }),
    startRecording: (conversationId) =>
      invoke<void>("start_recording", { conversationId }),
    stopRecording: () => invoke<void>("stop_recording"),
    listAudioDevices: () => invoke<string[]>("list_audio_devices"),
    onTranscriptMessage: (cb) =>
      listen<Message>("transcript-message", (e) => cb(e.payload)),
    onRecordingError: (cb) =>
      listen<string>("recording-error", (e) => cb(e.payload)),
  };
}

// --- Browser fallback (Vite preview, present window, future web) --------------
//
// Mirrors the SQLite store in localStorage so the UI is fully functional and
// persistent outside the desktop shell. First run seeds from mock data.

const LS_KEY = "kaigi.state.v1";

interface LocalState {
  conversations: Conversation[];
  messages: Record<string, Message[]>;
  settings: Partial<Settings> | null;
}

function readLocal(): LocalState {
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (raw) return JSON.parse(raw) as LocalState;
  } catch {
    /* ignore corrupt state, reseed below */
  }
  const seeded: LocalState = {
    conversations: MOCK_CONVERSATIONS,
    messages: MOCK_MESSAGES,
    settings: null,
  };
  writeLocal(seeded);
  return seeded;
}

function writeLocal(state: LocalState): void {
  try {
    localStorage.setItem(LS_KEY, JSON.stringify(state));
  } catch {
    /* storage full / unavailable — degrade to in-memory only */
  }
}

function localBackend(): Backend {
  const mutate = (fn: (s: LocalState) => void) => {
    const s = readLocal();
    fn(s);
    writeLocal(s);
    return Promise.resolve();
  };
  return {
    bootstrap: () => Promise.resolve(readLocal()),
    createConversation: (c) =>
      mutate((s) => {
        s.conversations.unshift(c);
        s.messages[c.id] = [];
      }),
    renameConversation: (id, title, updatedAt) =>
      mutate((s) => {
        const c = s.conversations.find((x) => x.id === id);
        if (c) {
          c.title = title;
          c.updatedAt = updatedAt;
        }
      }),
    setConversationLangs: (id, langA, langB, updatedAt) =>
      mutate((s) => {
        const c = s.conversations.find((x) => x.id === id);
        if (c) {
          c.langA = langA;
          c.langB = langB;
          c.updatedAt = updatedAt;
        }
      }),
    setSpeakerNames: (id, namesJson, updatedAt) =>
      mutate((s) => {
        const c = s.conversations.find((x) => x.id === id);
        if (c) {
          c.speakerNames = namesJson;
          c.updatedAt = updatedAt;
        }
      }),
    setMessageSpeaker: (messageId, label) =>
      mutate((s) => {
        for (const list of Object.values(s.messages)) {
          const m = list.find((x) => x.id === messageId);
          if (m) {
            m.speaker = label;
            break;
          }
        }
      }),
    deleteConversation: (id) =>
      mutate((s) => {
        s.conversations = s.conversations.filter((x) => x.id !== id);
        delete s.messages[id];
      }),
    addMessage: (m) =>
      mutate((s) => {
        const list = (s.messages[m.conversationId] ??= []);
        const i = list.findIndex((x) => x.id === m.id);
        if (i >= 0) list[i] = m;
        else list.push(m);
        const c = s.conversations.find((x) => x.id === m.conversationId);
        if (c) c.updatedAt = m.createdAt;
      }),
    saveSettings: (settings) =>
      mutate((s) => {
        s.settings = settings;
      }),
    translate: (text, from, to) =>
      browserTranslate(readLocal().settings, text, from, to),
    summarize: (conversationId, lang) => {
      const s = readLocal();
      const transcript = (s.messages[conversationId] ?? [])
        .map((m) =>
          m.translatedText
            ? `${m.originalText}\n${m.translatedText}`
            : m.originalText
        )
        .join("\n\n");
      return browserSummarize(s.settings, transcript, lang);
    },
    // Live mic capture lives in the Rust core; unavailable in the browser.
    startRecording: () =>
      Promise.reject(new Error("Recording is only available in the desktop app")),
    stopRecording: () => Promise.resolve(),
    listAudioDevices: () => Promise.resolve([]),
    onTranscriptMessage: () => Promise.resolve(() => {}),
    onRecordingError: () => Promise.resolve(() => {}),
  };
}

// --- Browser provider (mirrors Rust provider/ selection) ---------------------
//
// When settings select a usable API provider, call the OpenAI-compatible
// endpoint directly from the browser; otherwise echo like the Rust MockProvider
// so the UI stays functional without keys (docs/PROJECT.md §10.3).

function usesApi(settings: Partial<Settings> | null): settings is Settings {
  return (
    !!settings &&
    settings.providerMode === "api" &&
    !!settings.apiBaseUrl &&
    !!settings.apiKey
  );
}

async function chatCompletion(
  settings: Settings,
  system: string,
  user: string
): Promise<string> {
  const base = settings.apiBaseUrl.replace(/\/+$/, "");
  const resp = await fetch(`${base}/chat/completions`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${settings.apiKey}`,
    },
    body: JSON.stringify({
      model: settings.llmModel,
      temperature: 0.2,
      messages: [
        { role: "system", content: system },
        { role: "user", content: user },
      ],
    }),
  });
  if (!resp.ok) {
    throw new Error(`API error ${resp.status}: ${await resp.text()}`);
  }
  const data = await resp.json();
  return (data?.choices?.[0]?.message?.content ?? "").trim();
}

function browserTranslate(
  settings: Partial<Settings> | null,
  text: string,
  from: string,
  to: string
): Promise<string> {
  if (usesApi(settings)) {
    const system =
      `You are a professional translator. Translate the user's message from ` +
      `language '${from}' to language '${to}'. Preserve meaning, tone and ` +
      `register. Output ONLY the translation, with no quotes, labels or ` +
      `explanations.`;
    return chatCompletion(settings, system, text);
  }
  return Promise.resolve(`[${to.toUpperCase()}] ${text}`);
}

function browserSummarize(
  settings: Partial<Settings> | null,
  transcript: string,
  lang: string
): Promise<string> {
  if (usesApi(settings)) {
    const system =
      `Summarize the following bilingual conversation transcript in language ` +
      `'${lang}'. Produce concise markdown: a short overview, key points as ` +
      `bullets, and an 'Action items' list. Output only the markdown.`;
    return chatCompletion(settings, system, transcript);
  }
  return Promise.resolve(
    `## Summary (mock)\n\n- ${transcript.length} characters of transcript\n- ` +
      `Connect an API provider in Settings for a real summary.`
  );
}

// --- Selection ---------------------------------------------------------------

function hasTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

let cached: Backend | null = null;

export function getBackend(): Backend {
  if (cached) return cached;
  cached = hasTauri() ? tauriBackend() : localBackend();
  return cached;
}
