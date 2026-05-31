import { useApp } from "../state/AppState";
import { translate, type TKey } from "./index";

export function useT(): (key: TKey) => string {
  const { settings } = useApp();
  return (key) => translate(settings.appLanguage, key);
}
