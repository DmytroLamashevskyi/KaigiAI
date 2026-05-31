import type { Conversation, Message } from "../types";

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

interface CellProps {
  isOriginal: boolean;
  text: string;
  speaker?: string | null;
}

function Cell({ isOriginal, text, speaker }: CellProps) {
  if (!text) {
    return (
      <div className={"cell" + (isOriginal ? " original" : " translation")}>
        <span className="pending">…</span>
      </div>
    );
  }
  return (
    <div className={"cell" + (isOriginal ? " original" : " translation")}>
      {isOriginal && speaker && <span className="speaker-badge">{speaker}</span>}
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
        speaker={message.speaker}
      />
      <div className="row-time">{formatTime(message.startMs)}</div>
      <Cell
        isOriginal={!spokenOnA}
        text={spokenOnA ? message.translatedText : message.originalText}
        speaker={message.speaker}
      />
    </div>
  );
}
