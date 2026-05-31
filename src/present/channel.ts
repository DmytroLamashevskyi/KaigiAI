export const PRESENT_CHANNEL = "kaigi-present";

export interface PresentRow {
  a: string;
  b: string;
  speaker?: string | null;
  // Which side the utterance was originally spoken on — drives speaker
  // attribution and turn alignment in the presentation window.
  from: "A" | "B";
}

export interface PresentState {
  type: "state";
  locale: string;
  theme: "light" | "dark";
  fontScale: number;
  langAName: string;
  langBName: string;
  recording: boolean;
  rows: PresentRow[];
}

export interface PresentHello {
  type: "hello";
}

export type PresentMessage = PresentState | PresentHello;
