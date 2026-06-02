import { useApp } from "../../state/AppState";
import { useT } from "../../i18n/useT";
import { Field, LangSelect, Section } from "./common";

export default function LanguagesSection() {
  const { settings, updateSettings } = useApp();
  const t = useT();
  return (
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
  );
}
