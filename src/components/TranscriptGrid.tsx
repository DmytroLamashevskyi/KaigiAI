import { useEffect, useRef, type CSSProperties } from "react";
import { useApp, MAX_LANGS } from "../state/AppState";
import { getBackend } from "../backend";
import { logErr } from "../state/helpers";
import type { Conversation } from "../types";
import { conversationLangs, textForLang } from "../types";
import { languageName, LANGUAGES } from "../data/languages";
import { useT } from "../i18n/useT";
import { SpeakerBadge } from "./TranscriptRow";
import PendingRow from "./PendingRow";

/** N-column transcript for conversations with 3+ languages (§10.7). Each message
 *  is a card whose columns hold the same utterance in every conversation
 *  language; the column matching what was actually spoken is marked "original".
 *  For ≤2 languages the classic two-pane {@link TranscriptView} is used instead. */
export default function TranscriptGrid({ conversation }: { conversation: Conversation }) {
  const { activeMessages, activePending, recording, setLanguages } = useApp();
  const t = useT();
  const bodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bodyRef.current?.scrollTo({ top: bodyRef.current.scrollHeight });
  }, [activeMessages.length, activePending.length]);

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
    setLanguages(langs.filter((_, i) => i !== index));
  };
  const addLang = (code: string) => setLanguages([...langs, code]);

  // Two presentation windows, each shows the whole conversation and picks its
  // own language from a dropdown (§10.7) — so the slots aren't tied to a column.
  const openPresent = (slot: "A" | "B") =>
    getBackend()
      .openPresent(slot, languageName(langs[slot === "A" ? 0 : 1]))
      .catch(logErr("openPresent failed"));

  return (
    <div className="transcript-grid">
      <div className="grid-headers" style={cols}>
        {langs.map((lang, i) => (
          <div className="grid-head-cell" key={`${lang}-${i}`}>
            {locked ? (
              <span className="col-head">{languageName(lang)}</span>
            ) : (
              <select
                className="lang-select"
                value={lang}
                onChange={(e) => changeLang(i, e.target.value)}
              >
                {LANGUAGES.map((l) => (
                  <option
                    key={l.code}
                    value={l.code}
                    disabled={langs.includes(l.code) && l.code !== lang}
                  >
                    {l.nativeName}
                  </option>
                ))}
              </select>
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
        {langs.length < MAX_LANGS && (
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
        )}
        <span className="grid-toolbar-spacer" />
        <button className="present-btn" title={t("present.open")} onClick={() => openPresent("A")}>
          ⤢ 1
        </button>
        <button className="present-btn" title={t("present.open")} onClick={() => openPresent("B")}>
          ⤢ 2
        </button>
      </div>

      <div className="transcript-body grid-body" ref={bodyRef}>
        {activeMessages.length === 0 && activePending.length === 0 ? (
          <div className="placeholder transcript-empty">
            {recording ? t("view.listening") : t("view.startHint")}
          </div>
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
                      <span className="lang-badge">{languageName(m.detectedLang)}</span>
                    )}
                  </div>
                  {foreign && <div className="grid-msg-original">{m.originalText}</div>}
                  <div className="grid-cells" style={cols}>
                    {langs.map((lang) => {
                      const isOriginal = lang === m.detectedLang;
                      const text = textForLang(m, lang, conversation);
                      return (
                        <div
                          className={"grid-cell" + (isOriginal ? " original" : "")}
                          key={lang}
                        >
                          <div className="grid-cell-lang">{languageName(lang)}</div>
                          <div className="grid-cell-text">
                            {text || <span className="pending">…</span>}
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
