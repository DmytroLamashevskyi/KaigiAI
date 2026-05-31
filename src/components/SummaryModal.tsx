import { useEffect, useState } from "react";
import { useApp } from "../state/AppState";
import { useT } from "../i18n/useT";

export default function SummaryModal() {
  const { summaryOpen, closeSummary, summarize } = useApp();
  const t = useT();
  const [loading, setLoading] = useState(false);
  const [content, setContent] = useState("");
  const [failed, setFailed] = useState(false);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!summaryOpen) return;
    let cancelled = false;
    setLoading(true);
    setFailed(false);
    setContent("");
    setCopied(false);
    summarize()
      .then((text) => {
        if (!cancelled) setContent(text);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [summaryOpen]);

  if (!summaryOpen) return null;

  const copy = () => {
    navigator.clipboard
      ?.writeText(content)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  };

  return (
    <div className="modal-backdrop" onClick={closeSummary}>
      <div className="modal summary-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2>{t("summary.title")}</h2>
          <button className="close-btn" onClick={closeSummary} aria-label="Close">
            ✕
          </button>
        </div>
        <div className="modal-body">
          {loading ? (
            <p className="summary-status">{t("summary.loading")}</p>
          ) : failed ? (
            <p className="summary-status summary-failed">{t("summary.failed")}</p>
          ) : (
            <div className="summary-content">{content}</div>
          )}
        </div>
        {!loading && !failed && content && (
          <div className="modal-footer">
            <button className="summary-copy" onClick={copy}>
              {copied ? "✓ " : ""}
              {t("summary.copy")}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
