import type { Conversation } from "../types";
import { useApp } from "../state/AppState";
import { useT } from "../i18n/useT";

interface Props {
  conversation: Conversation;
  recording: boolean;
  onToggleRecording: () => void;
}

export default function LanguageBar({ conversation, recording, onToggleRecording }: Props) {
  const t = useT();
  const { activeMessages, openSummary } = useApp();

  return (
    <div className="lang-bar">
      <span className="conv-title-lg">{conversation.title}</span>

      <div className="lang-bar-actions">
        {activeMessages.length > 0 && (
          <button className="summary-btn" onClick={openSummary} title={t("summary.title")}>
            ✦ {t("summary.button")}
          </button>
        )}
        <button
          className={"record-btn" + (recording ? " recording" : "")}
          onClick={onToggleRecording}
        >
          <span className="rec-dot" />
          {recording ? t("rec.stop") : t("rec.start")}
        </button>
      </div>
    </div>
  );
}
