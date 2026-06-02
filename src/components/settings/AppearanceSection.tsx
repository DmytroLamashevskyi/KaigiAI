import { useApp } from "../../state/AppState";
import type { FontSize } from "../../types";
import { useT } from "../../i18n/useT";
import { Field, LangSelect, Section } from "./common";

export default function AppearanceSection() {
  const { settings, updateSettings } = useApp();
  const t = useT();
  const fontOptions: { id: FontSize; label: string }[] = [
    { id: "small", label: t("settings.fontSmall") },
    { id: "medium", label: t("settings.fontMedium") },
    { id: "large", label: t("settings.fontLarge") },
  ];

  return (
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
  );
}
