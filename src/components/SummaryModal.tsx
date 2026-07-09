import { useEffect, useState } from "react";
import { useApp } from "../state/AppState";
import { printHtmlViaIframe, summaryPrintHtml } from "../state/helpers";
import { useT } from "../i18n/useT";
import Modal from "./Modal";

export default function SummaryModal() {
  const { summaryOpen, closeSummary, summarize, activeConversation } = useApp();
  const t = useT();
  const [loading, setLoading] = useState(false);
  const [content, setContent] = useState("");
  const [failed, setFailed] = useState(false);
  const [copied, setCopied] = useState(false);
  // Frozen when the summary is generated: the user can switch conversations
  // while the modal is open, and the PDF must be titled after the conversation
  // that was actually summarized, not whichever is active at click time.
  const [pdfTitle, setPdfTitle] = useState("Конспект");

  useEffect(() => {
    if (!summaryOpen) return;
    let cancelled = false;
    setLoading(true);
    setFailed(false);
    setContent("");
    setCopied(false);
    setPdfTitle(activeConversation?.title ?? "Конспект");
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
    <Modal title={t("summary.title")} className="summary-modal" onClose={closeSummary}>
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
          <button
            className="summary-copy"
            title="Сохранить конспект как PDF через системную печать"
            onClick={() =>
              printHtmlViaIframe(summaryPrintHtml(`${pdfTitle} — конспект`, content))
            }
          >
            ⤓ PDF
          </button>
          <button className="summary-copy" onClick={copy}>
            {copied ? "✓ " : ""}
            {t("summary.copy")}
          </button>
        </div>
      )}
    </Modal>
  );
}
