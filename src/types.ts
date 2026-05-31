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
  startMs: number; // offset from recording start
  endMs: number;
  createdAt: number;
}

export interface Conversation {
  id: string;
  title: string;
  langA: LanguageCode;
  langB: LanguageCode;
  createdAt: number;
  updatedAt: number;
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
  nGpuLayers: number; // layers offloaded to GPU; 0 = CPU-only
  audioDevice: string; // empty = system default
  audioSource: AudioSource;
  saveAudio: boolean;
  fontSize: FontSize;
  theme: "light" | "dark";
}

export type View = "transcript" | "settings";
