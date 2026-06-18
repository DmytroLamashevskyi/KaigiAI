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
  theme: "light" | "dark";
  fontScale: number;
  /** Conversation languages in order; the present window picks one to display. */
  langs: string[];
  recording: boolean;
  rows: PresentRow[];
}

export interface PresentHello {
  type: "hello";
}

export type PresentMessage = PresentState | PresentHello;
