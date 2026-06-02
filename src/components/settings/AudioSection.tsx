import { useApp } from "../../state/AppState";
import { useT } from "../../i18n/useT";
import { Field, Section, useAudioInputs } from "./common";

export default function AudioSection() {
  const { settings, updateSettings } = useApp();
  const t = useT();
  const audioInputs = useAudioInputs();

  return (
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
      <Field
        label="Отсчёт до перевода"
        hint="После ~1,5 с тишины появляется полоска и отсчитывает это время до перевода фразы (0,5–3 с). Короткие паузы внутри речи её не показывают."
      >
        <div className="slider-row">
          <input
            type="range"
            className="settings-range"
            min={500}
            max={3000}
            step={100}
            value={settings.silenceMs}
            onChange={(e) => updateSettings({ silenceMs: Number(e.target.value) })}
          />
          <span className="slider-value">
            {(settings.silenceMs / 1000).toFixed(1)} с
          </span>
        </div>
      </Field>
      <Field
        label="Определять третий язык"
        hint="Выкл — речь всегда относится к одному из двух языков беседы (надёжнее, без ложных строк). Включи, только если реально говорят на третьем языке."
      >
        <button
          className={"switch" + (settings.detectForeignLanguages ? " on" : "")}
          onClick={() =>
            updateSettings({
              detectForeignLanguages: !settings.detectForeignLanguages,
            })
          }
        >
          <span className="switch-knob" />
        </button>
      </Field>
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
          onChange={(e) => updateSettings({ diarizationModelPath: e.target.value })}
        />
      </Field>
    </Section>
  );
}
