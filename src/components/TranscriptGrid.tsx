import { type CSSProperties } from "react";
import { useApp, MAX_LANGS } from "../state/AppState";
import { openPresentWindow } from "../state/helpers";
import type { Conversation } from "../types";
import { conversationLangs, textForLang } from "../types";
import { languageName, LANGUAGES } from "../data/languages";
import { isRtl } from "../i18n";
import { useAutoScroll } from "../hooks/useAutoScroll";
import { useT } from "../i18n/useT";
import { AddLangOffer, SpeakerBadge } from "./TranscriptRow";
import PendingRow, { TranscriptEmpty } from "./PendingRow";
import LangPicker from "./LangPicker";
import LangCodeSelect from "./LangCodeSelect";

/** N-column transcript for conversations with 3+ languages (§10.7). Each message
 *  is a card whose columns hold the same utterance in every conversation
 *  language; the column matching what was actually spoken is marked "original".
 *  For ≤2 languages the classic two-pane {@link TranscriptView} is used instead. */
export default function TranscriptGrid({ conversation }: { conversation: Conversation }) {
  const {
    activeMessages,
    activePending,
    recording,
    setLanguages,
    setMessageLang,
    translatingIds,
  } = useApp();
  const t = useT();
  const bodyRef = useAutoScroll<HTMLDivElement>([
    activeMessages.length,
    activePending.length,
  ]);

  const langs = conversationLangs(conversation);
  const locked = activeMessages.length > 0;
  const cols = { ["--cols" as string]: langs.length } as CSSProperties;

  const changeLang = (index: number, code: string) => {
    if (langs.includes(code)) return; // no duplicates
    const next = [...langs];
    next[index] = code;
    setLanguages(next);
  };
  const removeLang = (index: number) => {
    if (langs.length <= 2) return;
    // Stored `message_translation` rows for the dropped language are kept on
    // purpose: re-adding the language restores them (history isn't re-translated,
    // §10.7). An utterance spoken in the removed language simply becomes a
    // full-width "foreign" row, which is the correct semantics.
    setLanguages(langs.filter((_, i) => i !== index));
  };
  const addLang = (code: string) => setLanguages([...langs, code]);

  return (
    <div className="transcript-grid">
      <div className="grid-headers" style={cols}>
        {langs.map((lang, i) => (
          <div className="grid-head-cell" key={`${lang}-${i}`}>
            {locked ? (
              <span className="col-head">{languageName(lang)}</span>
            ) : (
              <LangCodeSelect
                value={lang}
                disabledCodes={langs.filter((c) => c !== lang)}
                onChange={(code) => changeLang(i, code)}
              />
            )}
            {langs.length > 2 && (
              <button
                type="button"
                className="lang-remove-btn"
                title="Убрать язык"
                onClick={() => removeLang(i)}
              >
                ×
              </button>
            )}
          </div>
        ))}
      </div>

      <div className="grid-toolbar">
        {langs.length < MAX_LANGS ? (
          <select
            className="lang-add-select"
            value=""
            onChange={(e) => e.target.value && addLang(e.target.value)}
            title="Добавить язык в разговор"
          >
            <option value="">＋ язык…</option>
            {LANGUAGES.filter((l) => !langs.includes(l.code)).map((l) => (
              <option key={l.code} value={l.code}>
                {l.nativeName}
              </option>
            ))}
          </select>
        ) : (
          <span className="lang-cap-hint" title="Предел языков на беседу">
            Максимум {MAX_LANGS} языков
          </span>
        )}
        <span className="grid-toolbar-spacer" />
        {/* Two presentation windows, each shows the whole conversation and picks
            its own language from a dropdown (§10.7) — the slots aren't tied to a
            column; the title is just the window's initial caption. */}
        <button
          className="present-btn"
          title={t("present.open")}
          onClick={() => openPresentWindow("A", languageName(langs[0]))}
        >
          ⤢ 1
        </button>
        <button
          className="present-btn"
          title={t("present.open")}
          onClick={() => openPresentWindow("B", languageName(langs[1]))}
        >
          ⤢ 2
        </button>
      </div>

      <div className="transcript-body grid-body" ref={bodyRef}>
        {activeMessages.length === 0 && activePending.length === 0 ? (
          <TranscriptEmpty recording={recording} />
        ) : (
          <>
            {activeMessages.map((m) => {
              const foreign = !langs.includes(m.detectedLang);
              return (
                <div className="grid-msg" key={m.id}>
                  <div className="grid-msg-meta">
                    <span title={new Date(m.createdAt).toLocaleString()}>
                      {new Date(m.createdAt).toLocaleTimeString()}
                    </span>
                    {m.speaker && (
                      <SpeakerBadge conv={conversation} label={m.speaker} messageId={m.id} />
                    )}
                    {foreign && (
                      <>
                        <span className="lang-badge">{languageName(m.detectedLang)}</span>
                        <AddLangOffer conv={conversation} lang={m.detectedLang} />
                      </>
                    )}
                    {/* ⇄: correct which language the utterance was spoken in
                        (N-language manual fix) — re-translates into the rest. */}
                    <LangPicker
                      className="lang-reassign"
                      triggerClassName="lang-reassign-btn"
                      label="⇄"
                      title="Указать язык реплики"
                      header="Язык реплики"
                      options={langs.filter((l) => l !== m.detectedLang)}
                      renderLabel={languageName}
                      onPick={(l) => setMessageLang(m.id, l)}
                    />
                  </div>
                  {foreign && (
                    <div
                      className="grid-msg-original"
                      dir={isRtl(m.detectedLang) ? "rtl" : "ltr"}
                    >
                      {m.originalText}
                    </div>
                  )}
                  <div className="grid-cells" style={cols}>
                    {langs.map((lang) => {
                      const isOriginal = lang === m.detectedLang;
                      const text = textForLang(m, lang, conversation);
                      return (
                        <div
                          className={"grid-cell" + (isOriginal ? " original" : "")}
                          key={lang}
                          dir={isRtl(lang) ? "rtl" : "ltr"}
                        >
                          <div className="grid-cell-lang">{languageName(lang)}</div>
                          <div className="grid-cell-text">
                            {text ||
                              (translatingIds[m.id] ? (
                                <span className="pending">…</span>
                              ) : (
                                <span className="grid-cell-empty">—</span>
                              ))}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>
              );
            })}
            {activePending.map((p) => (
              <PendingRow key={p.pendingId} pending={p} />
            ))}
          </>
        )}
      </div>
    </div>
  );
}
