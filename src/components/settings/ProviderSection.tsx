import { useState } from "react";
import { useApp } from "../../state/AppState";
import { LOCAL_DOWNLOADS } from "../../data/models";
import { useT } from "../../i18n/useT";
import { ApiPresets, deriveProviderModes, DownloadHint, Field, Section } from "./common";

const SCENARIOS: {
  id: string;
  title: string;
  desc: string;
  sttMode: "local" | "api";
  translationMode: "local" | "api";
}[] = [
  {
    id: "local",
    title: "💻 Всё локально",
    desc: "Речь и перевод на вашем ПК (whisper.cpp + llama.cpp). Приватно, без интернета, нужен GPU.",
    sttMode: "local",
    translationMode: "local",
  },
  {
    id: "mixed",
    title: "💻→☁ Локальная речь + облачный перевод",
    desc: "Речь распознаётся на ПК, перевод — через облако (напр. Gemini). Хороший перевод без мощного GPU под LLM.",
    sttMode: "local",
    translationMode: "api",
  },
  {
    id: "cloud",
    title: "☁ Всё через облако",
    desc: "И речь, и перевод — через API (напр. Groq). Без GPU, нужен только ключ. Данные уходят провайдеру.",
    sttMode: "api",
    translationMode: "api",
  },
];

export default function ProviderSection() {
  const { settings, updateSettings } = useApp();
  const t = useT();
  // STT and translation each run locally or on a cloud API. Rather than expose
  // the two raw toggles (confusing), we present three named scenarios; the raw
  // modes are derived from the picked card. The technical server paths live
  // under an "Advanced" disclosure that auto-opens when a needed path is blank.
  const { sttLocal, translationLocal, anyLocal, anyApi } =
    deriveProviderModes(settings);
  // The active scenario, derived from the two modes (the 4th combo — cloud
  // speech + local translation — is uncommon and simply highlights nothing;
  // NB the wizard's shorter expression highlights "cloud" for that combo — a
  // known divergence, kept as-is).
  const scenario =
    sttLocal && translationLocal
      ? "local"
      : sttLocal && !translationLocal
        ? "mixed"
        : !sttLocal && !translationLocal
          ? "cloud"
          : "";
  // Whether every server path the local stages need is filled in.
  const localComplete =
    (!sttLocal || (!!settings.localWhisperServerPath && !!settings.localWhisperPath)) &&
    (!translationLocal || (!!settings.localLlmServerPath && !!settings.localLlmPath));
  const [advancedOpen, setAdvancedOpen] = useState(false);
  // Show local path fields when the user expands Advanced, or force them visible
  // while required paths are still missing so first-time setup isn't hidden.
  const showLocalFields = anyLocal && (advancedOpen || !localComplete);

  return (
    <Section title={t("settings.aiProvider")}>
      <p className="field-hint" style={{ marginBottom: 4 }}>
        Выберите, как работать. Можно поменять в любой момент.
      </p>
      <div className="scenario-list">
        {SCENARIOS.map((sc) => (
          <button
            key={sc.id}
            type="button"
            className={"scenario-card" + (scenario === sc.id ? " active" : "")}
            onClick={() =>
              updateSettings({
                sttMode: sc.sttMode,
                translationMode: sc.translationMode,
              })
            }
          >
            <span className="scenario-title">{sc.title}</span>
            <span className="scenario-desc">{sc.desc}</span>
          </button>
        ))}
      </div>

      {anyLocal && (
        <Field
          label="Запускать сервис при старте приложения"
          hint="Вкл — серверы прогреваются сразу (первая запись мгновенная, но VRAM занят всё время). Выкл — стартуют при нажатии «Запись» (полоска ~10–15с)."
        >
          <button
            className={"switch" + (settings.startServersOnLaunch ? " on" : "")}
            onClick={() =>
              updateSettings({
                startServersOnLaunch: !settings.startServersOnLaunch,
              })
            }
          >
            <span className="switch-knob" />
          </button>
        </Field>
      )}

      {anyLocal && localComplete && (
        <button
          type="button"
          className="advanced-toggle"
          onClick={() => setAdvancedOpen((o) => !o)}
        >
          {advancedOpen ? "▾" : "▸"} Дополнительно: пути к серверам и моделям
        </button>
      )}

      {showLocalFields && (
        <>
          <p className="note-box">{t("settings.localServersHint")}</p>
          {sttLocal && (
            <>
              <Field label="whisper-server" hint="Путь к серверу whisper.cpp (.exe)">
                <input
                  className="settings-input"
                  value={settings.localWhisperServerPath}
                  placeholder="C:\\whisper.cpp\\whisper-server.exe"
                  onChange={(e) =>
                    updateSettings({ localWhisperServerPath: e.target.value })
                  }
                />
              </Field>
              <DownloadHint link={LOCAL_DOWNLOADS.whisperServer} />
              <Field label="Модель Whisper" hint="GGML .bin">
                <input
                  className="settings-input"
                  value={settings.localWhisperPath}
                  placeholder="C:\\models\\ggml-large-v3.bin"
                  onChange={(e) => updateSettings({ localWhisperPath: e.target.value })}
                />
              </Field>
              <DownloadHint link={LOCAL_DOWNLOADS.whisperModels} />
            </>
          )}
          {translationLocal && (
            <>
              <Field label="llama-server" hint="Путь к серверу llama.cpp (.exe)">
                <input
                  className="settings-input"
                  value={settings.localLlmServerPath}
                  placeholder="C:\\llama.cpp\\llama-server.exe"
                  onChange={(e) =>
                    updateSettings({ localLlmServerPath: e.target.value })
                  }
                />
              </Field>
              <DownloadHint link={LOCAL_DOWNLOADS.llamaServer} />
              <Field label="Модель LLM" hint="GGUF instruct-модель">
                <input
                  className="settings-input"
                  value={settings.localLlmPath}
                  placeholder="C:\\models\\qwen2.5-7b-instruct-q5_k_m.gguf"
                  onChange={(e) => updateSettings({ localLlmPath: e.target.value })}
                />
              </Field>
              <DownloadHint link={LOCAL_DOWNLOADS.llmModels} />
            </>
          )}
          <Field label="Слои на GPU" hint="0 = только CPU">
            <input
              className="settings-input"
              type="number"
              min={0}
              value={settings.nGpuLayers}
              onChange={(e) =>
                updateSettings({ nGpuLayers: Number(e.target.value) || 0 })
              }
            />
          </Field>
          <p className="note-box">{t("settings.localNote")}</p>
        </>
      )}

      {anyApi && (
        <>
          <div className="api-help">
            <p className="api-help-title">{t("settings.apiQuickSetup")}</p>
            <ApiPresets keyLabel={t("settings.getKey")} />
          </div>
          <Field label={t("settings.baseUrl")} hint={t("settings.baseUrlHint")}>
            <input
              className="settings-input"
              value={settings.apiBaseUrl}
              placeholder="https://api.groq.com/openai/v1"
              onChange={(e) => updateSettings({ apiBaseUrl: e.target.value })}
            />
          </Field>
          <Field label={t("settings.apiKey")}>
            <input
              className="settings-input"
              type="password"
              value={settings.apiKey}
              placeholder={t("settings.apiKeyPlaceholder")}
              onChange={(e) => updateSettings({ apiKey: e.target.value })}
            />
          </Field>
          {!sttLocal && (
            <>
              <Field
                label="Модель распознавания (Whisper)"
                hint="Для Groq: whisper-large-v3"
              >
                <input
                  className="settings-input"
                  value={settings.sttModel}
                  placeholder="whisper-large-v3"
                  onChange={(e) => updateSettings({ sttModel: e.target.value })}
                />
              </Field>
              <p className="note-box warn">
                Распознавание речи по API есть только у провайдеров с Whisper
                (напр. Groq). Gemini его не поддерживает — используйте Gemini
                только для перевода, а речь распознавайте локально или через Groq.
              </p>
            </>
          )}
          {!translationLocal && (
            <Field label="Модель перевода (LLM)">
              <input
                className="settings-input"
                value={settings.llmModel}
                placeholder="llama-3.3-70b-versatile"
                onChange={(e) => updateSettings({ llmModel: e.target.value })}
              />
            </Field>
          )}
          <p className="note-box warn">{t("settings.apiNote")}</p>
        </>
      )}
    </Section>
  );
}
