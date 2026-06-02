import { useEffect, useRef, useState } from "react";
import type { PresentState } from "../present/channel";
import { onPresentState, postPresentHello } from "../present/transport";
import { translate, isRtl } from "../i18n";

// Rendered in a standalone window (present.html, label present-a/present-b).
// Shows one side of the transcript large, for an audience/second screen, with
// its own toolbar (text size, theme, show-original) remembered per window.
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
    const off = onPresentState(setState);
    postPresentHello();
    const onFocus = () => postPresentHello();
    window.addEventListener("focus", onFocus);
    return () => {
      off();
      window.removeEventListener("focus", onFocus);
    };
  }, []);

  // Keep re-requesting until we actually receive state (startup race).
  useEffect(() => {
    if (state) return;
    const id = setInterval(postPresentHello, 1000);
    return () => clearInterval(id);
  }, [state]);

  const locale = state?.locale ?? "en";
  const name = side === "A" ? state?.langAName : state?.langBName;
  const langAName = state?.langAName ?? "A";
  const langBName = state?.langBName ?? "B";
  const recording = state?.recording ?? false;

  const turns = (state?.rows ?? [])
    .map((r) => ({
      text: side === "A" ? r.a : r.b,
      // The other side's text = the original utterance for a translated turn.
      source: side === "A" ? r.b : r.a,
      from: r.from,
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
        <span className="present-lang">{name}</span>
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
            const own = turn.from === side;
            const speakerName =
              turn.speaker || (turn.from === "A" ? langAName : langBName);
            return (
              <div key={i} className={"present-turn" + (own ? " own" : " other")}>
                <div className="present-speaker">{speakerName}</div>
                <p className="present-line">{turn.text}</p>
                {showOriginal && !own && turn.source && (
                  <p className="present-source">{turn.source}</p>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
