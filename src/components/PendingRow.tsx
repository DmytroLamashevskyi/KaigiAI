import { type CSSProperties } from "react";
import type { PendingSegment } from "../types";
import { useT } from "../i18n/useT";

/** Empty-transcript placeholder shared by the 2-pane view and the N-lang grid. */
export function TranscriptEmpty({ recording }: { recording: boolean }) {
  const t = useT();
  return (
    <div className="placeholder transcript-empty">
      {recording ? t("view.listening") : t("view.startHint")}
    </div>
  );
}

/** Full-width placeholder bar shown while an utterance settles (§10.8). In the
 *  `silence` phase a fill sweeps across the chat width over `hangoverMs` — the
 *  live "did they stop talking?" countdown; if they resume the row vanishes.
 *  Once the pause elapses it flips to a `processing` shimmer until the real text
 *  lands. No numeric timer — just the bar. */
export default function PendingRow({ pending }: { pending: PendingSegment }) {
  const isSilence = pending.phase === "silence";
  // Anchor the CSS animation to when this phase began, so a placeholder that
  // mounts mid-countdown (or re-renders) stays in sync with the backend clock.
  const elapsed = Date.now() - pending.since;
  const style = isSilence
    ? ({
        // Negative delay starts the fill already partway through.
        animationDuration: `${pending.hangoverMs ?? 3000}ms`,
        animationDelay: `${-elapsed}ms`,
      } as CSSProperties)
    : undefined;
  return (
    <div className="transcript-row pending-row">
      <div className={"pending-bar" + (isSilence ? " silence" : " processing")}>
        <span className="pending-bar-fill" style={style} />
      </div>
    </div>
  );
}
