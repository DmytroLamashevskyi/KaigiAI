import { useApp } from "../state/AppState";
import { useT } from "../i18n/useT";
import AppearanceSection from "./settings/AppearanceSection";
import LanguagesSection from "./settings/LanguagesSection";
import ProviderSection from "./settings/ProviderSection";
import AudioSection from "./settings/AudioSection";

/** Settings screen shell — composes the section components, each of which reads
 *  `settings`/`updateSettings` from the app context directly. */
export default function Settings() {
  const { closeSettings, openWizard } = useApp();
  const t = useT();

  return (
    <div className="settings-page">
      <div className="settings-header">
        <div className="settings-header-inner">
          <h1>{t("settings.title")}</h1>
          <div className="settings-header-actions">
            <button className="wizard-open-btn" onClick={openWizard}>
              ✦ Мастер настройки
            </button>
            <button className="close-btn" onClick={closeSettings}>
              ✕
            </button>
          </div>
        </div>
      </div>

      <div className="settings-scroll">
        <AppearanceSection />
        <LanguagesSection />
        <ProviderSection />
        <AudioSection />
      </div>
    </div>
  );
}
