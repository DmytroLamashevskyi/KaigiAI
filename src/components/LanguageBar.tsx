import type { Conversation } from "../types";
import { useT } from "../i18n/useT";

interface Props {
  conversation: Conversation;
  recording: boolean;
  onToggleRecording: () => void;
}

export default function LanguageBar({ conversation, recording, onToggleRecording }: Props) {
  const t = useT();

  return (
    <div className="lang-bar">
      <span className="conv-title-lg">{conversation.title}</span>

      <button
        className={"record-btn" + (recording ? " recording" : "")}
        onClick={onToggleRecording}
      >
        <span className="rec-dot" />
        {recording ? t("rec.stop") : t("rec.start")}
      </button>
    </div>
  );
}
