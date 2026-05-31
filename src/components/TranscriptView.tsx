import { useEffect, useRef } from "react";
import { useApp } from "../state/AppState";
import { languageName, LANGUAGES } from "../data/languages";
import { FONT_SCALE } from "../data/fontSize";
import { useT } from "../i18n/useT";
import LanguageBar from "./LanguageBar";
import TranscriptRow from "./TranscriptRow";
import InputBar from "./InputBar";

function openPresent(side: "A" | "B") {
  window.open(
    `?present=${side}`,
    `kaigiPresent${side}`,
    "width=900,height=640"
  );
}

export default function TranscriptView() {
  const {
    activeConversation,
    activeMessages,
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
  }, [activeMessages.length]);

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
            onClick={() => openPresent("A")}
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
            onClick={() => openPresent("B")}
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
        {activeMessages.length === 0 ? (
          <div className="placeholder transcript-empty">
            {recording ? t("view.listening") : t("view.startHint")}
          </div>
        ) : (
          activeMessages.map((m) => (
            <TranscriptRow key={m.id} message={m} conversation={activeConversation} />
          ))
        )}
      </div>

      <InputBar />
    </div>
  );
}
