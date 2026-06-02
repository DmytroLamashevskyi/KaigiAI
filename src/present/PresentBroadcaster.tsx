import { useEffect, useRef } from "react";
import { useApp } from "../state/AppState";
import { languageName } from "../data/languages";
import { FONT_SCALE } from "../data/fontSize";
import type { PresentState } from "./channel";
import { onPresentHello, postPresentState } from "./transport";

// Lives in the main window. Mirrors the active conversation to any open
// presentation windows over the cross-window transport (Tauri events / browser
// BroadcastChannel).
export default function PresentBroadcaster() {
  const { activeConversation, activeMessages, settings, recording } = useApp();
  const stateRef = useRef<PresentState | null>(null);

  const langA = activeConversation?.langA ?? "";
  const langB = activeConversation?.langB ?? "";
  stateRef.current = {
    type: "state",
    locale: settings.appLanguage,
    theme: settings.theme,
    fontScale: FONT_SCALE[settings.fontSize],
    langAName: languageName(langA),
    langBName: languageName(langB),
    recording,
    rows: activeMessages.map((m) => {
      const spokenOnA = m.detectedLang === langA;
      return {
        a: spokenOnA ? m.originalText : m.translatedText,
        b: spokenOnA ? m.translatedText : m.originalText,
        speaker: m.speaker ?? null,
        from: spokenOnA ? ("A" as const) : ("B" as const),
      };
    }),
  };

  // A present window that just opened asks for the current state; reply to it.
  useEffect(
    () => onPresentHello(() => {
      if (stateRef.current) postPresentState(stateRef.current);
    }),
    []
  );

  // Push every time the mirrored state changes.
  useEffect(() => {
    if (stateRef.current) postPresentState(stateRef.current);
  }, [activeConversation, activeMessages, settings, recording]);

  return null;
}
