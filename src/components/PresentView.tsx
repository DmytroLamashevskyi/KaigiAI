import { useEffect, useRef, useState } from "react";
import type { PresentState } from "../present/channel";
import { onPresentState, postPresentHello } from "../present/transport";
import { translate, isRtl } from "../i18n";

// Rendered in a standalone window (?present=A|B). Receives live updates over a
// BroadcastChannel and shows one side of the transcript, large, for an audience.
// Each utterance is attributed to the speaker who originally said it and aligned
// like a chat so the audience can follow who is saying what.
export default function PresentView({ side }: { side: "A" | "B" }) {
  const [state, setState] = useState<PresentState | null>(null);
  const bodyRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const off = onPresentState(setState);
    postPresentHello();
    return off;
  }, []);

  useEffect(() => {
    if (state) document.documentElement.setAttribute("data-theme", state.theme);
  }, [state?.theme]);

  const locale = state?.locale ?? "en";
  const name = side === "A" ? state?.langAName : state?.langBName;
  const langAName = state?.langAName ?? "A";
  const langBName = state?.langBName ?? "B";
  const scale = state?.fontScale ?? 1;
  const recording = state?.recording ?? false;

  const turns = (state?.rows ?? [])
    .map((r) => ({
      text: side === "A" ? r.a : r.b,
      from: r.from,
      speaker: r.speaker,
    }))
    .filter((r) => r.text);

  useEffect(() => {
    bodyRef.current?.scrollTo({ top: bodyRef.current.scrollHeight });
  }, [turns.length]);

  return (
    <div
      className="present-page"
      dir={isRtl(locale) ? "rtl" : "ltr"}
      style={{ ["--present-scale" as string]: scale }}
    >
      <div className="present-header">
        <span className="present-lang">{name}</span>
        <span className={"present-status" + (recording ? " live" : "")}>
          <span className="present-dot" />
          {recording
            ? translate(locale, "present.live")
            : translate(locale, "present.paused")}
        </span>
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
              <div
                key={i}
                className={"present-turn" + (own ? " own" : " other")}
              >
                <div className="present-speaker">{speakerName}</div>
                <p className="present-line">{turn.text}</p>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
