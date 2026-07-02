import { useApp, MAX_LANGS } from "../state/AppState";
import { openPresentWindow } from "../state/helpers";
import { conversationLangs } from "../types";
import { languageName, LANGUAGES } from "../data/languages";
import { FONT_SCALE } from "../data/fontSize";
import { useAutoScroll } from "../hooks/useAutoScroll";
import { useT } from "../i18n/useT";
import LanguageBar from "./LanguageBar";
import TranscriptRow from "./TranscriptRow";
import TranscriptGrid from "./TranscriptGrid";
import PendingRow, { TranscriptEmpty } from "./PendingRow";
import LangPicker from "./LangPicker";
import LangCodeSelect from "./LangCodeSelect";
import InputBar from "./InputBar";

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
  const bodyRef = useAutoScroll<HTMLDivElement>([
    activeMessages.length,
    activePending.length,
  ]);

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
            <LangCodeSelect
              className="lang-select lang-a"
              value={langA}
              disabledCodes={[langB]}
              onChange={(code) => setConversationLangs(code, langB)}
            />
          )}
          <button
            className="present-btn"
            title={t("present.open")}
            onClick={() => openPresentWindow("A", languageName(langA))}
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
              options={LANGUAGES.filter((l) => !langs.includes(l.code)).map(
                (l) => l.code
              )}
              renderLabel={(code) =>
                LANGUAGES.find((l) => l.code === code)?.nativeName ?? code
              }
              onPick={(code) => setLanguages([...langs, code])}
            />
          )}
        </div>

        <div className="col-head-cell b">
          <button
            className="present-btn"
            title={t("present.open")}
            onClick={() => openPresentWindow("B", languageName(langB))}
          >
            ⤢
          </button>
          {locked ? (
            <span className="col-head lang-b-text">{languageName(langB)}</span>
          ) : (
            <LangCodeSelect
              className="lang-select lang-b"
              value={langB}
              disabledCodes={[langA]}
              onChange={(code) => setConversationLangs(langA, code)}
            />
          )}
        </div>
      </div>

      <div className="transcript-body" ref={bodyRef}>
        {activeMessages.length === 0 && activePending.length === 0 ? (
          <TranscriptEmpty recording={recording} />
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
