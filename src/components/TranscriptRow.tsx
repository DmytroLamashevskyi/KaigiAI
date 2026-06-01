import { useState } from "react";
import type { Conversation, Message } from "../types";
import { displaySpeaker } from "../types";
import { languageName } from "../data/languages";
import { useApp } from "../state/AppState";

interface Props {
  message: Message;
  conversation: Conversation;
}

function formatTime(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const m = Math.floor(totalSec / 60);
  const s = totalSec % 60;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/** Distinct speaker labels seen in a conversation, ordered numerically so
 *  "Speaker 2" sorts before "Speaker 10". */
function speakerLabels(messages: Message[]): string[] {
  const set = new Set<string>();
  for (const m of messages) if (m.speaker) set.add(m.speaker);
  return [...set].sort((a, b) =>
    a.localeCompare(b, undefined, { numeric: true })
  );
}

/** Next free "Speaker N" label given the ones already in use. */
function nextSpeakerLabel(labels: string[]): string {
  let max = 0;
  for (const l of labels) {
    const m = /^Speaker (\d+)$/.exec(l);
    if (m) max = Math.max(max, parseInt(m[1], 10));
  }
  return `Speaker ${max + 1}`;
}

/** Speaker label on an utterance. Clicking opens a menu to either reassign this
 *  single utterance to another speaker (manual diarization fix, §10.9) or
 *  rename the speaker globally for the whole conversation (§10.6). */
function SpeakerBadge({
  conv,
  label,
  messageId,
}: {
  conv: Conversation;
  label: string;
  messageId: string;
}) {
  const { renameSpeaker, reassignSpeaker, activeMessages } = useApp();
  const [editing, setEditing] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  const name = displaySpeaker(conv, label) ?? label;

  if (editing) {
    return (
      <input
        className="speaker-edit"
        defaultValue={name}
        autoFocus
        onBlur={(e) => {
          renameSpeaker(label, e.target.value);
          setEditing(false);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
          else if (e.key === "Escape") setEditing(false);
        }}
      />
    );
  }

  const labels = speakerLabels(activeMessages);
  const others = labels.filter((l) => l !== label);

  const reassign = (target: string) => {
    reassignSpeaker(messageId, target);
    setMenuOpen(false);
  };

  return (
    <span className="speaker-badge-wrap">
      <button
        type="button"
        className="speaker-badge"
        title="Reassign or rename speaker"
        onClick={() => setMenuOpen((v) => !v)}
      >
        {name}
      </button>
      {menuOpen && (
        <>
          <div className="speaker-menu-backdrop" onClick={() => setMenuOpen(false)} />
          <div className="speaker-menu" role="menu">
            {others.length > 0 && (
              <>
                <div className="speaker-menu-label">Reassign to</div>
                {others.map((l) => (
                  <button
                    key={l}
                    type="button"
                    className="speaker-menu-item"
                    onClick={() => reassign(l)}
                  >
                    {displaySpeaker(conv, l) ?? l}
                  </button>
                ))}
              </>
            )}
            <button
              type="button"
              className="speaker-menu-item"
              onClick={() => reassign(nextSpeakerLabel(labels))}
            >
              + New speaker
            </button>
            <div className="speaker-menu-sep" />
            <button
              type="button"
              className="speaker-menu-item"
              onClick={() => {
                setMenuOpen(false);
                setEditing(true);
              }}
            >
              Rename “{name}”…
            </button>
          </div>
        </>
      )}
    </span>
  );
}

interface CellProps {
  isOriginal: boolean;
  text: string;
  conversation: Conversation;
  speaker?: string | null;
  messageId: string;
}

function Cell({ isOriginal, text, conversation, speaker, messageId }: CellProps) {
  if (!text) {
    return (
      <div className={"cell" + (isOriginal ? " original" : " translation")}>
        <span className="pending">…</span>
      </div>
    );
  }
  return (
    <div className={"cell" + (isOriginal ? " original" : " translation")}>
      {isOriginal && speaker && (
        <SpeakerBadge conv={conversation} label={speaker} messageId={messageId} />
      )}
      {isOriginal && <span className="orig-dot" />}
      <span className="cell-text">{text}</span>
    </div>
  );
}

export default function TranscriptRow({ message, conversation }: Props) {
  const { langA, langB } = conversation;
  // A "foreign" utterance was spoken in neither pair language (docs/PROJECT.md
  // §10.7, variant A): show the original full-width with a language badge, then
  // a translation into each pair language side by side.
  const isForeign =
    message.detectedLang !== langA && message.detectedLang !== langB;

  if (isForeign) {
    return (
      <div className="transcript-row foreign">
        <div className="foreign-original cell original">
          {message.speaker && (
            <SpeakerBadge
              conv={conversation}
              label={message.speaker}
              messageId={message.id}
            />
          )}
          <span className="lang-badge">{languageName(message.detectedLang)}</span>
          <span className="cell-text">{message.originalText}</span>
        </div>
        <div className="foreign-translations">
          <Cell
            isOriginal={false}
            text={message.translatedText}
            conversation={conversation}
            messageId={message.id}
          />
          <div className="row-time">{formatTime(message.startMs)}</div>
          <Cell
            isOriginal={false}
            text={message.translatedTextB ?? ""}
            conversation={conversation}
            messageId={message.id}
          />
        </div>
      </div>
    );
  }

  const spokenOnA = message.detectedLang === langA;

  return (
    <div className="transcript-row">
      <Cell
        isOriginal={spokenOnA}
        text={spokenOnA ? message.originalText : message.translatedText}
        conversation={conversation}
        speaker={message.speaker}
        messageId={message.id}
      />
      <div className="row-time">{formatTime(message.startMs)}</div>
      <Cell
        isOriginal={!spokenOnA}
        text={spokenOnA ? message.translatedText : message.originalText}
        conversation={conversation}
        speaker={message.speaker}
        messageId={message.id}
      />
    </div>
  );
}
