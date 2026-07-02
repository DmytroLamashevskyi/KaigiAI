import { useEffect, useRef } from "react";
import { useApp } from "../state/AppState";
import { conversationLangs, textForLang } from "../types";
import type { PresentState } from "./channel";
import { onPresentHello, postPresentState } from "./transport";

// Lives in the main window. Mirrors the active conversation to any open
// presentation windows over the cross-window transport (Tauri events / browser
// BroadcastChannel). Carries every conversation language so each present window
// can pick which one to display (§10.7).
export default function PresentBroadcaster() {
  const { activeConversation, activeMessages, settings, recording } = useApp();
  const stateRef = useRef<PresentState | null>(null);

  const langs = activeConversation ? conversationLangs(activeConversation) : [];
  stateRef.current = {
    type: "state",
    locale: settings.appLanguage,
    langs,
    recording,
    rows: activeConversation
      ? activeMessages.map((m) => {
          // Original under its own language, plus a translation per other language.
          const texts: Record<string, string> = { [m.detectedLang]: m.originalText };
          for (const lang of langs) {
            texts[lang] = textForLang(m, lang, activeConversation);
          }
          return {
            texts,
            fromLang: m.detectedLang,
            speaker: m.speaker ?? null,
          };
        })
      : [],
  };

  // A present window that just opened asks for the current state; reply to it.
  useEffect(
    () => onPresentHello(() => {
      if (stateRef.current) postPresentState(stateRef.current);
    }),
    []
  );

  // Push every time the mirrored state changes. Depends only on the settings
  // field the payload actually carries (appLanguage) — depending on the whole
  // settings object would re-broadcast the entire transcript over the Tauri
  // event bus on every keystroke in a Settings text field.
  useEffect(() => {
    if (stateRef.current) postPresentState(stateRef.current);
  }, [activeConversation, activeMessages, settings.appLanguage, recording]);

  return null;
}
