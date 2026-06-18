import { useEffect, useRef, useState } from "react";
import type { PresentState } from "../present/channel";
import { onPresentState, postPresentHello } from "../present/transport";
import { languageName } from "../data/languages";
import { translate, isRtl } from "../i18n";

// Rendered in a standalone window (present.html, label present-a/present-b).
// Shows one conversation language large, for an audience/second screen, with its
// own toolbar (language, text size, theme, show-original) remembered per window.
const SCALE_MIN = 0.7;
const SCALE_MAX = 2.6;
const SCALE_STEP = 0.1;

export default function PresentView({ side }: { side: "A" | "B" }) {
  const [state, setState] = useState<PresentState | null>(null);
  const bodyRef = useRef<HTMLDivElement>(null);

  // Per-window controls, persisted in localStorage (each screen independent).
  const lsKey = (k: string) => `present.${side}.${k}`;
  const [scale, setScale] = useState(
    () => Number(localStorage.getItem(lsKey("scale"))) || 1.3
  );
  const [theme, setTheme] = useState<"light" | "dark">(
    () => (localStorage.getItem(lsKey("theme")) as "light" | "dark") || "light"
  );
  const [showOriginal, setShowOriginal] = useState(
    () => localStorage.getItem(lsKey("orig")) !== "0"
  );
  // Which conversation language this window shows. Persisted; defaults to this
  // window's slot (A→first language, B→second) once state arrives (§10.7).
  const [lang, setLang] = useState<string>(
    () => localStorage.getItem(lsKey("lang")) || ""
  );

  useEffect(() => localStorage.setItem(lsKey("scale"), String(scale)), [scale]);
  useEffect(() => {
    localStorage.setItem(lsKey("theme"), theme);
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);
  useEffect(
    () => localStorage.setItem(lsKey("orig"), showOriginal ? "1" : "0"),
    [showOriginal]
  );
  useEffect(() => {
    if (lang) localStorage.setItem(lsKey("lang"), lang);
  }, [lang]);

  useEffect(() => {
    const off = onPresentState(setState);
    postPresentHello();
    const onFocus = () => postPresentHello();
    window.addEventListener("focus", onFocus);
    return () => {
      off();
      window.removeEventListener("focus", onFocus);
    };
  }, []);

  // Settle on a language once the conversation's list arrives: a remembered one
  // if still valid, otherwise this window's slot (A→[0], B→[1]).
  const langsList = state?.langs ?? [];
  useEffect(() => {
    if (langsList.length === 0) return;
    if (lang && langsList.includes(lang)) return;
    setLang(langsList[side === "A" ? 0 : 1] ?? langsList[0]);
  }, [langsList.join(","), side]);

  // Keep re-requesting until we actually receive state (covers the startup race
  // where the broadcaster wasn't listening yet). Capped so it can't poll forever
  // if the broadcaster is truly gone (focusing the window re-arms it anyway).
  useEffect(() => {
    if (state) return;
    let attempts = 0;
    const id = setInterval(() => {
      postPresentHello();
      attempts += 1;
      if (attempts >= 30) clearInterval(id);
    }, 1000);
    return () => clearInterval(id);
  }, [state]);

  const locale = state?.locale ?? "en";
  const name = lang ? languageName(lang) : "";
  const recording = state?.recording ?? false;

  const turns = (state?.rows ?? [])
    .map((r) => ({
      text: r.texts[lang] ?? "",
      // The original utterance, shown small beneath a translated line.
      source: r.texts[r.fromLang] ?? "",
      own: r.fromLang === lang,
      from: r.fromLang,
      speaker: r.speaker,
    }))
    .filter((r) => r.text);

  useEffect(() => {
    bodyRef.current?.scrollTo({ top: bodyRef.current.scrollHeight });
  }, [turns.length]);

  const bump = (d: number) =>
    setScale((s) => Math.min(SCALE_MAX, Math.max(SCALE_MIN, +(s + d).toFixed(2))));

  return (
    <div
      className="present-page"
      dir={isRtl(locale) ? "rtl" : "ltr"}
      style={{ ["--present-scale" as string]: scale }}
    >
      <div className="present-header">
        {langsList.length > 2 ? (
          <select
            className="present-lang-select"
            value={lang}
            onChange={(e) => setLang(e.target.value)}
            title="Язык этого окна"
          >
            {langsList.map((l) => (
              <option key={l} value={l}>
                {languageName(l)}
              </option>
            ))}
          </select>
        ) : (
          <span className="present-lang">{name}</span>
        )}
        <div className="present-controls">
          <span
            className={"present-status" + (recording ? " live" : "")}
            title={recording ? "Идёт запись" : "Пауза"}
          >
            <span className="present-dot" />
            {recording
              ? translate(locale, "present.live")
              : translate(locale, "present.paused")}
          </span>
          <button className="present-ctl" title="Меньше текст" onClick={() => bump(-SCALE_STEP)}>
            A−
          </button>
          <button className="present-ctl" title="Больше текст" onClick={() => bump(SCALE_STEP)}>
            A+
          </button>
          <button
            className={"present-ctl" + (showOriginal ? " on" : "")}
            title="Показывать оригинал реплики"
            onClick={() => setShowOriginal((v) => !v)}
          >
            ориг.
          </button>
          <button
            className="present-ctl"
            title="Светлая / тёмная тема"
            onClick={() => setTheme((t) => (t === "dark" ? "light" : "dark"))}
          >
            {theme === "dark" ? "☀" : "🌙"}
          </button>
        </div>
      </div>
      <div className="present-body" ref={bodyRef}>
        {turns.length === 0 ? (
          <div className="present-empty">{translate(locale, "present.waiting")}</div>
        ) : (
          turns.map((turn, i) => {
            const speakerName = turn.speaker || languageName(turn.from);
            return (
              <div key={i} className={"present-turn" + (turn.own ? " own" : " other")}>
                <div className="present-speaker">{speakerName}</div>
                {/* Each line follows the script direction of its own language,
                    independent of the UI locale (§10.7 RTL support). */}
                <p className="present-line" dir={isRtl(lang) ? "rtl" : "ltr"}>
                  {turn.text}
                </p>
                {showOriginal && !turn.own && turn.source && (
                  <p className="present-source" dir={isRtl(turn.from) ? "rtl" : "ltr"}>
                    {turn.source}
                  </p>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
