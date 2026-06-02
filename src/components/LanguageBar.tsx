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
  const { activeMessages, openSummary, preparing } = useApp();

  return (
    <div className="lang-bar">
      <span className="conv-title-lg">{conversation.title}</span>

      <div className="lang-bar-actions">
        {activeMessages.length > 0 && (
          <button
            className="summary-btn prominent"
            onClick={openSummary}
            title={t("summary.title")}
          >
            ✦ {t("summary.button")}
          </button>
        )}
        <button
          className={
            "record-btn" +
            (recording ? " recording" : "") +
            (preparing ? " preparing" : "")
          }
          onClick={onToggleRecording}
          disabled={preparing}
        >
          {preparing ? (
            <>
              <span className="rec-spinner" />
              Подготовка сервиса…
            </>
          ) : (
            <>
              <span className="rec-dot" />
              {recording ? t("rec.stop") : t("rec.start")}
            </>
          )}
        </button>
      </div>
      {preparing && (
        <div className="prepare-bar" title="Запуск локальных серверов (загрузка модели)">
          <span className="prepare-fill" />
        </div>
      )}
    </div>
  );
}
