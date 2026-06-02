import { useEffect, useRef, type CSSProperties } from "react";
import { useApp } from "../state/AppState";
import { getBackend } from "../backend";
import { logErr } from "../state/helpers";
import type { PendingSegment } from "../types";
import { languageName, LANGUAGES } from "../data/languages";
import { FONT_SCALE } from "../data/fontSize";
import { useT } from "../i18n/useT";
import LanguageBar from "./LanguageBar";
import TranscriptRow from "./TranscriptRow";
import InputBar from "./InputBar";

/** Full-width placeholder bar shown while an utterance settles (§10.8). In the
 *  `silence` phase a fill sweeps across the chat width over `hangoverMs` — the
 *  live "did they stop talking?" countdown; if they resume the row vanishes.
 *  Once the pause elapses it flips to a `processing` shimmer until the real text
 *  lands. No numeric timer — just the bar. */
function PendingRow({ pending }: { pending: PendingSegment }) {
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

function openPresent(side: "A" | "B", title: string) {
  // Routes through the backend: a native Tauri window in the desktop app, or
  // window.open in the browser (window.open doesn't work in the Tauri webview).
  // `title` is the language name for the window caption.
  getBackend().openPresent(side, title).catch(logErr("openPresent failed"));
}

export default function TranscriptView() {
  const {
    activeConversation,
    activeMessages,
    activePending,
    settings,
    recording,
    toggleRecording,
    setConversationLangs,
    swapLanguages,
  } = useApp();
  const t = useT();
  const bodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bodyRef.current?.scrollTo({ top: bodyRef.current.scrollHeight });
  }, [activeMessages.length, activePending.length]);

  if (!activeConversation) {
    return <div className="placeholder">{t("view.pickOrCreate")}</div>;
  }

  const locked = activeMessages.length > 0;
  const { langA, langB } = activeConversation;
  const scale = FONT_SCALE[settings.fontSize];

  return (
    <div className="transcript-view" style={{ ["--fs-scale" as string]: scale }}>
      <LanguageBar
        conversation={activeConversation}
        recording={recording}
        onToggleRecording={toggleRecording}
      />

      <div className="col-headers">
        <div className="col-head-cell a">
          {locked ? (
            <span className="col-head lang-a-text">{languageName(langA)}</span>
          ) : (
            <select
              className="lang-select lang-a"
              value={langA}
              onChange={(e) => setConversationLangs(e.target.value, langB)}
            >
              {LANGUAGES.map((l) => (
                <option key={l.code} value={l.code} disabled={l.code === langB}>
                  {l.nativeName}
                </option>
              ))}
            </select>
          )}
          <button
            className="present-btn"
            title={t("present.open")}
            onClick={() => openPresent("A", languageName(langA))}
          >
            ⤢
          </button>
        </div>

        <div className="col-head-mid">
          {!locked && (
            <button className="swap-btn" onClick={swapLanguages} title={t("rec.swap")}>
              ⇄
            </button>
          )}
        </div>

        <div className="col-head-cell b">
          <button
            className="present-btn"
            title={t("present.open")}
            onClick={() => openPresent("B", languageName(langB))}
          >
            ⤢
          </button>
          {locked ? (
            <span className="col-head lang-b-text">{languageName(langB)}</span>
          ) : (
            <select
              className="lang-select lang-b"
              value={langB}
              onChange={(e) => setConversationLangs(langA, e.target.value)}
            >
              {LANGUAGES.map((l) => (
                <option key={l.code} value={l.code} disabled={l.code === langA}>
                  {l.nativeName}
                </option>
              ))}
            </select>
          )}
        </div>
      </div>

      <div className="transcript-body" ref={bodyRef}>
        {activeMessages.length === 0 && activePending.length === 0 ? (
          <div className="placeholder transcript-empty">
            {recording ? t("view.listening") : t("view.startHint")}
          </div>
        ) : (
          <>
            {activeMessages.map((m) => (
              <TranscriptRow key={m.id} message={m} conversation={activeConversation} />
            ))}
            {activePending.map((p) => (
              <PendingRow key={p.pendingId} pending={p} />
            ))}
          </>
        )}
      </div>

      <InputBar />
    </div>
  );
}
