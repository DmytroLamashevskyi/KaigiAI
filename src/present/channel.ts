export const PRESENT_CHANNEL = "kaigi-present";

export interface PresentRow {
  /** Utterance text per conversation language (code → text), including the
   *  original under its own `fromLang` key (§10.7). The present window renders
   *  whichever language its dropdown selects. */
  texts: Record<string, string>;
  /** Language the utterance was originally spoken in — drives speaker
   *  attribution, "own vs other" alignment, and the source line. */
  fromLang: string;
  speaker?: string | null;
}

export interface PresentState {
  type: "state";
  locale: string;
  /** Conversation languages in order; the present window picks one to display.
   *  Theme and text size are NOT carried here — each present window owns its
   *  own toggles, persisted per window in localStorage. */
  langs: string[];
  recording: boolean;
  rows: PresentRow[];
}
