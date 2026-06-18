import { useEffect, useRef } from "react";
import { useApp, MAX_LANGS } from "../state/AppState";
import { getBackend } from "../backend";
import { logErr } from "../state/helpers";
import { conversationLangs } from "../types";
import { languageName, LANGUAGES } from "../data/languages";
import { FONT_SCALE } from "../data/fontSize";
import { useT } from "../i18n/useT";
import LanguageBar from "./LanguageBar";
import TranscriptRow from "./TranscriptRow";
import TranscriptGrid from "./TranscriptGrid";
import PendingRow from "./PendingRow";
import LangPicker from "./LangPicker";
import InputBar from "./InputBar";

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
    setLanguages,
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
  const langs = conversationLangs(activeConversation);

  // 3+ languages render in a dedicated N-column grid (§10.7).
  if (langs.length > 2) {
    return (
      <div className="transcript-view" style={{ ["--fs-scale" as string]: scale }}>
        <LanguageBar
          conversation={activeConversation}
          recording={recording}
          onToggleRecording={toggleRecording}
        />
        <TranscriptGrid conversation={activeConversation} />
        <InputBar />
      </div>
    );
  }

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
          {langs.length < MAX_LANGS && (
            <LangPicker
              exclude={langs}
              onPick={(code) => setLanguages([...langs, code])}
            />
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
