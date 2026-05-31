import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import { useApp } from "../state/AppState";
import { getBackend } from "../backend";
import { LANGUAGES } from "../data/languages";
import { API_PROVIDERS, LOCAL_DOWNLOADS, type DownloadLink } from "../data/models";
import type { FontSize } from "../types";
import { useT } from "../i18n/useT";

interface AudioInput {
  value: string;
  label: string;
}

// On desktop the dropdown must list cpal device *names* — that is what the Rust
// capture pipeline looks up (audio/capture.rs `pick_input_device`). A browser
// `deviceId` would never match. We therefore prefer the native `list_audio_devices`
// command and only fall back to the Web API (labelled, needs mic permission) when
// running in a plain browser (dev preview), where capture is unavailable anyway.
function useAudioInputs() {
  const [devices, setDevices] = useState<AudioInput[]>([]);
  useEffect(() => {
    const backend = getBackend();
    let cancelled = false;
    let detachBrowser: (() => void) | undefined;

    const loadBrowser = () => {
      if (!navigator.mediaDevices?.enumerateDevices) return;
      const enumerate = () =>
        navigator.mediaDevices
          .enumerateDevices()
          .then((all) => {
            if (cancelled) return;
            setDevices(
              all
                .filter((d) => d.kind === "audioinput")
                .map((d) => ({ value: d.deviceId, label: d.label }))
            );
          })
          .catch(() => {});
      // Labels stay blank until mic access is granted; request it once.
      navigator.mediaDevices
        .getUserMedia({ audio: true })
        .then((stream) => {
          stream.getTracks().forEach((t) => t.stop());
          return enumerate();
        })
        .catch(() => enumerate());
      navigator.mediaDevices.addEventListener?.("devicechange", enumerate);
      detachBrowser = () =>
        navigator.mediaDevices.removeEventListener?.("devicechange", enumerate);
    };

    backend
      .listAudioDevices()
      .then((names) => {
        if (cancelled) return;
        if (names.length > 0) {
          setDevices(names.map((n) => ({ value: n, label: n })));
        } else {
          loadBrowser();
        }
      })
      .catch(() => loadBrowser());

    return () => {
      cancelled = true;
      detachBrowser?.();
    };
  }, []);
  return devices;
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="settings-section">
      <h2 className="section-title">{title}</h2>
      <div className="section-body">{children}</div>
    </section>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="field">
      <div className="field-label">
        <span>{label}</span>
        {hint && <span className="field-hint">{hint}</span>}
      </div>
      <div className="field-control">{children}</div>
    </div>
  );
}

function DownloadHint({ link }: { link: DownloadLink }) {
  return (
    <div className="dl-hint">
      <a className="api-key-link" href={link.url} target="_blank" rel="noreferrer">
        ↓ {link.label}
      </a>
      <span className="field-hint">{link.note}</span>
    </div>
  );
}

function LangSelect({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  return (
    <select className="settings-select" value={value} onChange={(e) => onChange(e.target.value)}>
      {LANGUAGES.map((l) => (
        <option key={l.code} value={l.code}>
          {l.nativeName} ({l.name})
        </option>
      ))}
    </select>
  );
}

export default function Settings() {
  const { settings, updateSettings, closeSettings } = useApp();
  const t = useT();
  const isApi = settings.providerMode === "api";
  const audioInputs = useAudioInputs();

  const fontOptions: { id: FontSize; label: string }[] = [
    { id: "small", label: t("settings.fontSmall") },
    { id: "medium", label: t("settings.fontMedium") },
    { id: "large", label: t("settings.fontLarge") },
  ];

  return (
    <div className="settings-page">
      <div className="settings-header">
        <div className="settings-header-inner">
          <h1>{t("settings.title")}</h1>
          <button className="close-btn" onClick={closeSettings}>
            ✕
          </button>
        </div>
      </div>

      <div className="settings-scroll">
        <Section title={t("settings.appearance")}>
          <Field label={t("settings.theme")}>
            <div className="seg">
              <button
                className={"seg-opt" + (settings.theme === "light" ? " active" : "")}
                onClick={() => updateSettings({ theme: "light" })}
              >
                {t("settings.themeLight")}
              </button>
              <button
                className={"seg-opt" + (settings.theme === "dark" ? " active" : "")}
                onClick={() => updateSettings({ theme: "dark" })}
              >
                {t("settings.themeDark")}
              </button>
            </div>
          </Field>
          <Field label={t("settings.fontSize")}>
            <div className="seg">
              {fontOptions.map((o) => (
                <button
                  key={o.id}
                  className={"seg-opt" + (settings.fontSize === o.id ? " active" : "")}
                  onClick={() => updateSettings({ fontSize: o.id })}
                >
                  {o.label}
                </button>
              ))}
            </div>
          </Field>
          <Field label={t("settings.uiLanguage")}>
            <LangSelect
              value={settings.appLanguage}
              onChange={(v) => updateSettings({ appLanguage: v })}
            />
          </Field>
        </Section>

        <Section title={t("settings.defaultLangs")}>
          <Field label={t("settings.leftPanel")}>
            <LangSelect
              value={settings.defaultLangA}
              onChange={(v) => updateSettings({ defaultLangA: v })}
            />
          </Field>
          <Field label={t("settings.rightPanel")}>
            <LangSelect
              value={settings.defaultLangB}
              onChange={(v) => updateSettings({ defaultLangB: v })}
            />
          </Field>
        </Section>

        <Section title={t("settings.aiProvider")}>
          <Field label={t("settings.mode")} hint={t("settings.modeHint")}>
            <div className="seg">
              <button
                className={"seg-opt" + (!isApi ? " active" : "")}
                onClick={() => updateSettings({ providerMode: "local" })}
              >
                💻 {t("settings.local")}
              </button>
              <button
                className={"seg-opt" + (isApi ? " active" : "")}
                onClick={() => updateSettings({ providerMode: "api" })}
              >
                ☁ {t("settings.api")}
              </button>
            </div>
          </Field>

          {!isApi ? (
            <>
              <p className="note-box">{t("settings.localServersHint")}</p>
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
          ) : (
            <>
              <div className="api-help">
                <p className="api-help-title">{t("settings.apiQuickSetup")}</p>
                <div className="api-presets">
                  {API_PROVIDERS.map((p) => (
                    <button
                      key={p.id}
                      className="api-preset"
                      onClick={() => updateSettings({ apiBaseUrl: p.baseUrl })}
                    >
                      <span className="api-preset-name">{p.name}</span>
                      <span className="api-preset-note">{p.note}</span>
                      {p.keyUrl && (
                        <a
                          className="api-key-link"
                          href={p.keyUrl}
                          target="_blank"
                          rel="noreferrer"
                          onClick={(e) => e.stopPropagation()}
                        >
                          {t("settings.getKey")}
                        </a>
                      )}
                    </button>
                  ))}
                </div>
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
              <Field label={t("settings.modelName")}>
                <input
                  className="settings-input"
                  value={settings.llmModel}
                  placeholder="llama-3.3-70b-versatile"
                  onChange={(e) => updateSettings({ llmModel: e.target.value })}
                />
              </Field>
              <p className="note-box warn">{t("settings.apiNote")}</p>
            </>
          )}
        </Section>

        <Section title={t("settings.audio")}>
          <Field label={t("settings.audioSource")}>
            <div className="seg">
              <button
                className={"seg-opt" + (settings.audioSource === "mic" ? " active" : "")}
                onClick={() => updateSettings({ audioSource: "mic" })}
              >
                🎤 {t("settings.sourceMic")}
              </button>
              <button
                className={"seg-opt" + (settings.audioSource === "system" ? " active" : "")}
                onClick={() => updateSettings({ audioSource: "system" })}
              >
                🔊 {t("settings.sourceSystem")}
              </button>
            </div>
          </Field>
          {settings.audioSource === "mic" && (
            <Field label={t("settings.inputDevice")} hint={t("settings.inputDeviceHint")}>
              <select
                className="settings-select"
                value={settings.audioDevice}
                onChange={(e) => updateSettings({ audioDevice: e.target.value })}
              >
                <option value="">{t("settings.defaultDevice")}</option>
                {audioInputs.map((d, i) => (
                  <option key={d.value || i} value={d.value}>
                    {d.label || `${t("settings.sourceMic")} ${i + 1}`}
                  </option>
                ))}
              </select>
            </Field>
          )}
          <Field label={t("settings.saveAudio")} hint={t("settings.saveAudioHint")}>
            <button
              className={"switch" + (settings.saveAudio ? " on" : "")}
              onClick={() => updateSettings({ saveAudio: !settings.saveAudio })}
            >
              <span className="switch-knob" />
            </button>
          </Field>
          <Field
            label={t("settings.diarizationModel")}
            hint={t("settings.diarizationModelHint")}
          >
            <input
              className="settings-input"
              value={settings.diarizationModelPath}
              placeholder="C:\\models\\voxceleb_resnet34.onnx"
              onChange={(e) =>
                updateSettings({ diarizationModelPath: e.target.value })
              }
            />
          </Field>
        </Section>
      </div>
    </div>
  );
}
