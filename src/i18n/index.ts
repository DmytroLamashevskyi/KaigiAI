import { translations, RTL_LANGS, type TKey } from "./strings";

export type { TKey };

export function translate(locale: string, key: TKey): string {
  const dict = translations[locale] ?? translations.en;
  return dict[key] ?? translations.en[key] ?? key;
}

export function isRtl(locale: string): boolean {
  return RTL_LANGS.has(locale);
}
