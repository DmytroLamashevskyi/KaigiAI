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
  startMs: number; // offset from recording start
  endMs: number;
  createdAt: number;
}

export interface Conversation {
  id: string;
  title: string;
  langA: LanguageCode;
  langB: LanguageCode;
  speakerNames?: string | null; // JSON map of diarization label -> display name
  createdAt: number;
  updatedAt: number;
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
  providerMode: ProviderMode;
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
  saveAudio: boolean;
  fontSize: FontSize;
  theme: "light" | "dark";
}

export type View = "transcript" | "settings";
