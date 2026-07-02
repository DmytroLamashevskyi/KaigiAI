import { useEffect, useState, type ReactNode } from "react";
import { getBackend } from "../../backend";
import { LANGUAGES } from "../../data/languages";
import { API_PROVIDERS, type DownloadLink } from "../../data/models";
import { logErr } from "../../state/helpers";
import type { Settings } from "../../types";
import { useApp } from "../../state/AppState";

/** Which provider tiers are in play for the current mode split — shared by the
 *  settings section and the first-run wizard so the two can't drift. */
export function deriveProviderModes(
  s: Pick<Settings, "sttMode" | "translationMode">
): { sttLocal: boolean; translationLocal: boolean; anyLocal: boolean; anyApi: boolean } {
  const sttLocal = s.sttMode === "local";
  const translationLocal = s.translationMode === "local";
  return {
    sttLocal,
    translationLocal,
    anyLocal: sttLocal || translationLocal,
    anyApi: !sttLocal || !translationLocal,
  };
}

/** The API-provider quick-setup cards (preset button + "get key" link), shared
 *  verbatim by ProviderSection and the first-run wizard. `keyLabel` differs:
 *  the settings section uses the i18n string, the wizard hardcodes Russian. */
export function ApiPresets({ keyLabel }: { keyLabel: string }) {
  const { updateSettings } = useApp();
  return (
    <div className="api-presets">
      {API_PROVIDERS.map((p) => (
        <div key={p.id} className="api-preset">
          <button
            type="button"
            className="api-preset-select"
            onClick={() => updateSettings({ apiBaseUrl: p.baseUrl })}
          >
            <span className="api-preset-name">{p.name}</span>
            <span className="api-preset-note">{p.note}</span>
          </button>
          {p.keyUrl && (
            <button
              type="button"
              className="api-key-link"
              onClick={() => openExternal(p.keyUrl)}
            >
              {keyLabel}
            </button>
          )}
        </div>
      ))}
    </div>
  );
}

export interface AudioInput {
  value: string;
  label: string;
}

// On desktop the dropdown must list cpal device *names* — that is what the Rust
// capture pipeline looks up (audio/capture.rs `pick_input_device`). A browser
// `deviceId` would never match. We therefore prefer the native `list_audio_devices`
// command and only fall back to the Web API (labelled, needs mic permission) when
// running in a plain browser (dev preview), where capture is unavailable anyway.
export function useAudioInputs(): AudioInput[] {
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

export function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="settings-section">
      <h2 className="section-title">{title}</h2>
      <div className="section-body">{children}</div>
    </section>
  );
}

export function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
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

/** Open an external URL in the system browser. A bare <a target=_blank> doesn't
 *  reach the OS browser from the Tauri webview, so all external links go through
 *  the backend `openUrl` command (browser fallback uses window.open). */
export function openExternal(url: string): void {
  getBackend().openUrl(url).catch(logErr("openUrl failed"));
}

export function DownloadHint({ link }: { link: DownloadLink }) {
  return (
    <div className="dl-hint">
      <button type="button" className="api-key-link" onClick={() => openExternal(link.url)}>
        ↓ {link.label}
      </button>
      <span className="field-hint">{link.note}</span>
    </div>
  );
}

export function LangSelect({
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
