import { useState } from "react";
import type { Conversation, Message } from "../types";
import { displaySpeaker } from "../types";
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

/** Speaker label that can be renamed in place; the new name persists for every
 *  utterance of that speaker in the conversation. */
function SpeakerBadge({ conv, label }: { conv: Conversation; label: string }) {
  const { renameSpeaker } = useApp();
  const [editing, setEditing] = useState(false);
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
  return (
    <button
      type="button"
      className="speaker-badge"
      title="Rename speaker"
      onClick={() => setEditing(true)}
    >
      {name}
    </button>
  );
}

interface CellProps {
  isOriginal: boolean;
  text: string;
  conversation: Conversation;
  speaker?: string | null;
}

function Cell({ isOriginal, text, conversation, speaker }: CellProps) {
  if (!text) {
    return (
      <div className={"cell" + (isOriginal ? " original" : " translation")}>
        <span className="pending">…</span>
      </div>
    );
  }
  return (
    <div className={"cell" + (isOriginal ? " original" : " translation")}>
      {isOriginal && speaker && <SpeakerBadge conv={conversation} label={speaker} />}
      {isOriginal && <span className="orig-dot" />}
      <span className="cell-text">{text}</span>
    </div>
  );
}

export default function TranscriptRow({ message, conversation }: Props) {
  const spokenOnA = message.detectedLang === conversation.langA;

  return (
    <div className="transcript-row">
      <Cell
        isOriginal={spokenOnA}
        text={spokenOnA ? message.originalText : message.translatedText}
        conversation={conversation}
        speaker={message.speaker}
      />
      <div className="row-time">{formatTime(message.startMs)}</div>
      <Cell
        isOriginal={!spokenOnA}
        text={spokenOnA ? message.translatedText : message.originalText}
        conversation={conversation}
        speaker={message.speaker}
      />
    </div>
  );
}
