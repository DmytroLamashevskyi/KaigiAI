export type LanguageCode = string;

export interface Language {
  code: LanguageCode;
  name: string; // English name
  nativeName: string;
}

export type MessageSource = "mic" | "system" | "text";

export interface Message {
  id: string;
  conversationId: string;
  source: MessageSource;
  detectedLang: LanguageCode; // language of the original (= which panel it was spoken on)
  speaker?: string | null; // populated once diarization exists
  originalText: string;
  translatedText: string;
  /** Secondary translation, only set for "foreign" rows whose `detectedLang`
   *  falls outside the conversation pair (docs/PROJECT.md §10.7, variant A):
   *  `translatedText` holds the langA translation, this holds langB. Null/absent
   *  for ordinary bilingual rows. */
  translatedTextB?: string | null;
  /** Per-language translations (langCode → text) for the N-language mode (§10.7).
   *  Holds a translation for every conversation language *except* the original
   *  (`detectedLang`, whose text is `originalText`). Empty for 2-language rows,
   *  which use `translatedText`/`translatedTextB`. The N-column grid reads this. */
  translations?: Record<string, string>;
  startMs: number; // offset from recording start
  endMs: number;
  createdAt: number;
  /** Speech→text pipeline latency in ms (STT + translation), §10.8. Absent for
   *  text messages and rows created before the feature existed. */
  processingMs?: number | null;
}

/** A placeholder row in the transcript while an utterance settles (§10.8).
 *  Lifecycle: `silence` (the speaker paused — a bar fills over `hangoverMs`; if
 *  they resume in time the segment-cancelled event drops it) → `processing`
 *  (the pause elapsed, STT/translation are running — a shimmer) → replaced by
 *  the real message, or cancelled. */
export interface PendingSegment {
  pendingId: number;
  conversationId: string;
  phase: "silence" | "processing";
  /** Hangover duration the silence bar should fill over (ms); silence phase only. */
  hangoverMs?: number;
  /** Wall-clock ms when the current phase began, so the CSS bar can be anchored. */
  since: number;
}

export interface Conversation {
  id: string;
  title: string;
  langA: LanguageCode;
  langB: LanguageCode;
  /** Ordered conversation languages for the N-language mode (§10.7). For a
   *  2-language chat this is `[langA, langB]`; with 3+ it drives the N-column
   *  grid. Absent on rows that predate the feature — use {@link conversationLangs}
   *  which falls back to `[langA, langB]`. Order = UI column order. */
  langs?: LanguageCode[];
  speakerNames?: string | null; // JSON map of diarization label -> display name
  createdAt: number;
  updatedAt: number;
}

/** A conversation's ordered language list, falling back to `[langA, langB]` for
 *  older rows that predate the multi-language column (§10.7). */
export function conversationLangs(conv: Conversation): LanguageCode[] {
  return conv.langs && conv.langs.length > 0
    ? conv.langs
    : [conv.langA, conv.langB];
}

/** The text of a message in a given conversation language: the original when
 *  `lang` is what was spoken, otherwise its translation. Falls back to the
 *  legacy `translatedText`/`translatedTextB` columns for 2-language rows that
 *  predate the per-language `translations` map (§10.7). */
export function textForLang(
  message: Message,
  lang: LanguageCode,
  conv: Conversation
): string {
  if (lang === message.detectedLang) return message.originalText;
  const fromMap = message.translations?.[lang];
  if (fromMap !== undefined) return fromMap;
  // Legacy fallback for messages saved before the per-language `translations`
  // map. The scalar columns only ever held the original *pair* languages
  // (langA/langB), so a later-added 3rd language has no stored text → "".
  const foreign =
    message.detectedLang !== conv.langA && message.detectedLang !== conv.langB;
  if (foreign) {
    if (lang === conv.langA) return message.translatedText;
    if (lang === conv.langB) return message.translatedTextB ?? "";
    return "";
  }
  // Pair message: `translatedText` is the translation into the *other* pair
  // language; any other language is unknown.
  const other = message.detectedLang === conv.langA ? conv.langB : conv.langA;
  return lang === other ? message.translatedText : "";
}

/** Parsed `speakerNames` JSON (label -> display name); empty on absent/invalid. */
export function speakerMap(conv: Conversation): Record<string, string> {
  if (!conv.speakerNames) return {};
  try {
    return JSON.parse(conv.speakerNames) as Record<string, string>;
  } catch {
    return {};
  }
}

/** Resolve a diarization label to its user-given name, or the label itself. */
export function displaySpeaker(
  conv: Conversation,
  label?: string | null
): string | null {
  if (!label) return null;
  return speakerMap(conv)[label] ?? label;
}

export type ProviderMode = "local" | "api";
export type AudioSource = "mic" | "system";
export type FontSize = "small" | "medium" | "large";

export interface Settings {
  appLanguage: LanguageCode;
  defaultLangA: LanguageCode;
  defaultLangB: LanguageCode;
  /** Where speech recognition runs. Independent from translation so a user can
   *  pair local whisper with a cloud translator (e.g. Gemini, which has no STT
   *  endpoint). Migrated from the old single `providerMode`. */
  sttMode: ProviderMode;
  /** Where translation/summary runs. */
  translationMode: ProviderMode;
  /** Pre-start the local servers on app launch so the first recording is
   *  instant. Off = start lazily on the Record button (frees VRAM until used). */
  startServersOnLaunch: boolean;
  apiBaseUrl: string;
  apiKey: string;
  sttModel: string;
  llmModel: string;
  localWhisperServerPath: string; // path to whisper.cpp server executable (local mode)
  localLlmServerPath: string; // path to llama.cpp server executable (local mode)
  localWhisperPath: string; // GGML/GGUF whisper model (local mode)
  localLlmPath: string; // GGUF instruct model (local mode)
  diarizationModelPath: string; // ONNX speaker-embedding model; empty = diarization off
  nGpuLayers: number; // layers offloaded to GPU; 0 = CPU-only
  audioDevice: string; // empty = system default
  audioSource: AudioSource;
  /** Visible countdown of the silence bar (ms), 500–3000. A fixed ~1.5 s grace
   *  precedes it; total hangover = grace + this (§10.8). */
  silenceMs: number;
  /** Allow utterances in a third language (outside the pair) to be detected and
   *  shown as full-width "foreign" rows (§10.7 variant A). Off → every utterance
   *  is forced onto langA/langB, avoiding spurious mislabelled rows. */
  detectForeignLanguages: boolean;
  /** Folder where ZIP exports are written (empty → a default under app data). */
  exportDir: string;
  saveAudio: boolean;
  fontSize: FontSize;
  theme: "light" | "dark";
  /** Set once the first-run setup wizard has been completed/dismissed, so it
   *  doesn't auto-open again. */
  onboarded: boolean;
}

/** One unmet setup requirement for the current mode (from the readiness check),
 *  used by the "needs setup" banner and the first-run wizard. */
export interface SetupIssue {
  field: string;
  message: string;
}

export type View = "transcript" | "settings";
