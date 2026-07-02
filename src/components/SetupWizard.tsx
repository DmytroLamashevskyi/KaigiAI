import { useEffect, useState } from "react";
import { useApp } from "../state/AppState";
import { getBackend } from "../backend";
import { LOCAL_DOWNLOADS, type DownloadLink } from "../data/models";
import { ApiPresets, deriveProviderModes, DownloadHint } from "./settings/common";
import Modal from "./Modal";

/** A path input with a live "file exists" check (green ✓ / red ✗) plus an
 *  optional download link, so the user can see at a glance what's still missing. */
function PathField({
  label,
  value,
  placeholder,
  onChange,
  link,
}: {
  label: string;
  value: string;
  placeholder: string;
  onChange: (v: string) => void;
  link?: DownloadLink;
}) {
  const [exists, setExists] = useState<boolean | null>(null);
  useEffect(() => {
    let cancelled = false;
    if (!value.trim()) {
      setExists(null);
      return;
    }
    const id = setTimeout(() => {
      getBackend()
        .pathExists(value)
        .then((r) => !cancelled && setExists(r))
        .catch(() => {});
    }, 300);
    return () => {
      cancelled = true;
      clearTimeout(id);
    };
  }, [value]);

  return (
    <div className="wiz-field">
      <div className="wiz-field-label">
        <span>{label}</span>
        {exists === true && <span className="wiz-ok">✓ найден</span>}
        {exists === false && <span className="wiz-bad">✗ не найден</span>}
      </div>
      <input
        className="settings-input"
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
      />
      {link && <DownloadHint link={link} />}
    </div>
  );
}

export default function SetupWizard() {
  const { wizardOpen, closeWizard, finishOnboarding, settings, updateSettings, setupIssues } =
    useApp();
  const [step, setStep] = useState<"welcome" | "config">("welcome");

  if (!wizardOpen) return null;

  const { sttLocal, translationLocal, anyLocal, anyApi } =
    deriveProviderModes(settings);
  const ready = setupIssues.length === 0;

  const pickScenario = (stt: "local" | "api", tr: "local" | "api") =>
    updateSettings({ sttMode: stt, translationMode: tr });

  // NB: for the config-file-only combo sttMode="api"+translationMode="local"
  // this highlights "cloud" while ProviderSection highlights nothing — a known
  // divergence, kept as-is (unifying it would be a behavior change).
  const scenario =
    sttLocal && translationLocal ? "local" : sttLocal ? "mixed" : "cloud";

  return (
    <Modal title="Добро пожаловать в KaigiAI" className="wizard-modal" onClose={closeWizard}>
        {step === "welcome" ? (
          <div className="modal-body wizard-body">
            <p className="wizard-lead">
              Живой двуязычный перевод речи. Настроим за минуту — выберите, как
              работать: через облачный API (нужен только ключ) или локально на
              вашем ПК (приватно, нужен GPU и модели).
            </p>
            <div className="wizard-actions">
              <button className="wiz-primary" onClick={() => setStep("config")}>
                Поехали →
              </button>
            </div>
          </div>
        ) : (
          <div className="modal-body wizard-body">
            <p className="wizard-step-title">1. Как работать</p>
            <div className="scenario-list">
              <button
                type="button"
                className={"scenario-card" + (scenario === "cloud" ? " active" : "")}
                onClick={() => pickScenario("api", "api")}
              >
                <span className="scenario-title">☁ Через облако (проще)</span>
                <span className="scenario-desc">
                  Речь и перевод через API (напр. Groq). Без GPU, нужен только ключ.
                </span>
              </button>
              <button
                type="button"
                className={"scenario-card" + (scenario === "mixed" ? " active" : "")}
                onClick={() => pickScenario("local", "api")}
              >
                <span className="scenario-title">💻→☁ Локальная речь + облачный перевод</span>
                <span className="scenario-desc">
                  Речь на ПК (whisper), перевод через облако (напр. Gemini).
                </span>
              </button>
              <button
                type="button"
                className={"scenario-card" + (scenario === "local" ? " active" : "")}
                onClick={() => pickScenario("local", "local")}
              >
                <span className="scenario-title">💻 Всё локально</span>
                <span className="scenario-desc">
                  Полностью на ПК (whisper + llama). Приватно, нужен GPU и модели.
                </span>
              </button>
            </div>

            {anyApi && (
              <>
                <p className="wizard-step-title">2. Облако: ключ</p>
                <ApiPresets keyLabel="Получить ключ →" />
                <div className="wiz-field">
                  <div className="wiz-field-label">
                    <span>Base URL</span>
                  </div>
                  <input
                    className="settings-input"
                    value={settings.apiBaseUrl}
                    placeholder="https://api.groq.com/openai/v1"
                    onChange={(e) => updateSettings({ apiBaseUrl: e.target.value })}
                  />
                </div>
                <div className="wiz-field">
                  <div className="wiz-field-label">
                    <span>API-ключ</span>
                  </div>
                  <input
                    className="settings-input"
                    type="password"
                    value={settings.apiKey}
                    placeholder="вставьте ключ"
                    onChange={(e) => updateSettings({ apiKey: e.target.value })}
                  />
                </div>
              </>
            )}

            {anyLocal && (
              <>
                <p className="wizard-step-title">{anyApi ? "3" : "2"}. Локально: серверы и модели</p>
                <p className="wizard-hint">
                  Скачайте по ссылкам и укажите пути к файлам — зелёная галочка
                  значит «файл найден».
                </p>
                {sttLocal && (
                  <>
                    <PathField
                      label="whisper-server (.exe)"
                      value={settings.localWhisperServerPath}
                      placeholder="C:\\whisper.cpp\\whisper-server.exe"
                      onChange={(v) => updateSettings({ localWhisperServerPath: v })}
                      link={LOCAL_DOWNLOADS.whisperServer}
                    />
                    <PathField
                      label="Модель Whisper (GGML .bin)"
                      value={settings.localWhisperPath}
                      placeholder="C:\\models\\ggml-large-v3-q5_0.bin"
                      onChange={(v) => updateSettings({ localWhisperPath: v })}
                      link={LOCAL_DOWNLOADS.whisperModels}
                    />
                  </>
                )}
                {translationLocal && (
                  <>
                    <PathField
                      label="llama-server (.exe)"
                      value={settings.localLlmServerPath}
                      placeholder="C:\\llama.cpp\\llama-server.exe"
                      onChange={(v) => updateSettings({ localLlmServerPath: v })}
                      link={LOCAL_DOWNLOADS.llamaServer}
                    />
                    <PathField
                      label="Модель LLM (GGUF)"
                      value={settings.localLlmPath}
                      placeholder="C:\\models\\qwen2.5-7b-instruct-q5_k_m.gguf"
                      onChange={(v) => updateSettings({ localLlmPath: v })}
                      link={LOCAL_DOWNLOADS.llmModels}
                    />
                  </>
                )}
              </>
            )}

            <div className="wizard-actions">
              <button className="wiz-secondary" onClick={() => setStep("welcome")}>
                ← Назад
              </button>
              {ready ? (
                <button className="wiz-primary" onClick={finishOnboarding}>
                  Готово ✓
                </button>
              ) : (
                <>
                  <span className="wizard-remaining">
                    Осталось: {setupIssues.map((i) => i.message).join("; ")}
                  </span>
                  <button className="wiz-secondary" onClick={finishOnboarding}>
                    Пропустить
                  </button>
                </>
              )}
            </div>
          </div>
        )}
    </Modal>
  );
}
