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
  segmentationModel: DownloadLink;
} = {
  segmentationModel: {
    label: "Модель сегментации (pyannote, ONNX)",
    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2",
    note: "Распакуйте архив и укажите путь к model.onnx.",
  },
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
  /** Recommended models applied when the preset is picked — the app's defaults
   *  are local-mode names that don't exist on cloud providers (a Groq request
   *  with the default llmModel 404s). Absent = leave the user's value alone. */
  sttModel?: string;
  llmModel?: string;
}

export const API_PROVIDERS: ApiProviderInfo[] = [
  {
    id: "groq",
    name: "Groq",
    baseUrl: "https://api.groq.com/openai/v1",
    keyUrl: "https://console.groq.com/keys",
    note: "Бесплатный тир, очень быстрый. Хостит Whisper large-v3 и быстрые LLM.",
    // Verified against api.groq.com/openai/v1/models (2026-07).
    sttModel: "whisper-large-v3",
    llmModel: "llama-3.3-70b-versatile",
  },
  {
    id: "gemini",
    name: "Google Gemini",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
    keyUrl: "https://aistudio.google.com/app/apikey",
    note: "Бесплатный тир (Gemini Flash). Хорош для перевода.",
    // Gemini has no Whisper endpoint — leave sttModel alone (the UI warns).
    llmModel: "gemini-2.5-flash",
  },
  {
    id: "openai",
    name: "OpenAI-совместимый (свой сервер)",
    baseUrl: "http://localhost:11434/v1",
    keyUrl: "",
    note: "Ollama / LM Studio / vLLM / корпоративный сервер — укажите base URL.",
    // Models depend on what the user's server hosts — don't touch them.
  },
];
