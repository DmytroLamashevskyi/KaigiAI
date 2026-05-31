import { useEffect, useRef } from "react";
import { useApp } from "../state/AppState";
import { languageName } from "../data/languages";
import { FONT_SCALE } from "../data/fontSize";
import { PRESENT_CHANNEL, type PresentState } from "./channel";

// Lives in the main window. Mirrors the active conversation to any open
// presentation windows over a BroadcastChannel.
export default function PresentBroadcaster() {
  const { activeConversation, activeMessages, settings, recording } = useApp();
  const chanRef = useRef<BroadcastChannel | null>(null);
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

  useEffect(() => {
    const ch = new BroadcastChannel(PRESENT_CHANNEL);
    ch.onmessage = (e) => {
      if (e.data?.type === "hello" && stateRef.current) {
        ch.postMessage(stateRef.current);
      }
    };
    chanRef.current = ch;
    return () => ch.close();
  }, []);

  useEffect(() => {
    chanRef.current?.postMessage(stateRef.current);
  }, [activeConversation, activeMessages, settings, recording]);

  return null;
}
