// Catalog for the local model manager (download-at-first-run, per design 10.1).
// VRAM tiers and provider recommendations mirror docs/PROJECT.md.

export interface ModelInfo {
  id: string;
  label: string;
  sizeGb: number;
  tier: "low" | "mid" | "high";
  license: string;
}

export const STT_MODELS: ModelInfo[] = [
  { id: "whisper-small", label: "Whisper small", sizeGb: 1.2, tier: "low", license: "MIT" },
  { id: "whisper-medium", label: "Whisper medium", sizeGb: 3, tier: "mid", license: "MIT" },
  { id: "whisper-large-v3", label: "Whisper large-v3", sizeGb: 5, tier: "high", license: "MIT" },
];

export const LLM_MODELS: ModelInfo[] = [
  { id: "qwen2.5-3b-instruct", label: "Qwen2.5 3B Instruct", sizeGb: 2.5, tier: "low", license: "Qwen" },
  { id: "qwen2.5-7b-instruct", label: "Qwen2.5 7B Instruct", sizeGb: 5, tier: "mid", license: "Apache-2.0" },
  { id: "qwen2.5-14b-instruct", label: "Qwen2.5 14B Instruct", sizeGb: 9, tier: "high", license: "Apache-2.0" },
  { id: "gemma-2-9b-it", label: "Gemma 2 9B IT", sizeGb: 6, tier: "high", license: "Gemma" },
];

// Where to download the on-device servers and models for local mode. Shown as
// helper links next to the path fields in Settings so users can install
// everything without leaving the app. (Links open externally.)
export interface DownloadLink {
  label: string;
  url: string;
  note: string;
}

export const LOCAL_DOWNLOADS: {
  whisperServer: DownloadLink;
  llamaServer: DownloadLink;
  whisperModels: DownloadLink;
  llmModels: DownloadLink;
} = {
  whisperServer: {
    label: "whisper.cpp (whisper-server)",
    url: "https://github.com/ggml-org/whisper.cpp/releases",
    note: "Готовые сборки для Windows; нужен whisper-server.exe (CUDA-сборка для GPU).",
  },
  llamaServer: {
    label: "llama.cpp (llama-server)",
    url: "https://github.com/ggml-org/llama.cpp/releases",
    note: "Готовые сборки для Windows; нужен llama-server.exe (CUDA-сборка для GPU).",
  },
  whisperModels: {
    label: "Модели Whisper (GGML .bin)",
    url: "https://huggingface.co/ggerganov/whisper.cpp/tree/main",
    note: "Напр. ggml-large-v3.bin для лучшего качества.",
  },
  llmModels: {
    label: "Модели LLM (GGUF)",
    url: "https://huggingface.co/Qwen/Qwen2.5-7B-Instruct-GGUF/tree/main",
    note: "Напр. Qwen2.5-7B-Instruct Q5_K_M для перевода/саммари.",
  },
};

export interface ApiProviderInfo {
  id: string;
  name: string;
  baseUrl: string;
  keyUrl: string;
  note: string;
}

export const API_PROVIDERS: ApiProviderInfo[] = [
  {
    id: "groq",
    name: "Groq",
    baseUrl: "https://api.groq.com/openai/v1",
    keyUrl: "https://console.groq.com/keys",
    note: "Бесплатный тир, очень быстрый. Хостит Whisper large-v3 и быстрые LLM.",
  },
  {
    id: "gemini",
    name: "Google Gemini",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    keyUrl: "https://aistudio.google.com/app/apikey",
    note: "Бесплатный тир (Gemini Flash). Хорош для перевода.",
  },
  {
    id: "openai",
    name: "OpenAI-совместимый (свой сервер)",
    baseUrl: "http://localhost:11434/v1",
    keyUrl: "",
    note: "Ollama / LM Studio / vLLM / корпоративный сервер — укажите base URL.",
  },
];
