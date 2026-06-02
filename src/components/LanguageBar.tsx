import { useState } from "react";
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
  const { activeMessages, openSummary, preparing, autoTitle } = useApp();
  const [titling, setTitling] = useState(false);

  const runAutoTitle = () => {
    if (titling || activeMessages.length === 0) return;
    setTitling(true);
    autoTitle().finally(() => setTitling(false));
  };

  return (
    <div className="lang-bar">
      <div className="lang-bar-title">
        <span className="conv-title-lg">{conversation.title}</span>
        {activeMessages.length > 0 && (
          <button
            className="auto-title-btn"
            onClick={runAutoTitle}
            disabled={titling}
            title="Сгенерировать заголовок по разговору"
          >
            {titling ? "…" : "✨ Авто-заголовок"}
          </button>
        )}
      </div>

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
