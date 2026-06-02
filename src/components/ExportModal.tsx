import { useApp } from "../state/AppState";

/** Export picker: choose Markdown (downloads a .md file) or PDF (opens the
 *  system print dialog → "Save as PDF", which renders Japanese/Cyrillic with
 *  real system fonts). Opened from the conversation menu in the sidebar. */
export default function ExportModal() {
  const { exportId, closeExport, exportMarkdown, exportPdf, exportZip, conversations } =
    useApp();
  if (!exportId) return null;
  const conv = conversations.find((c) => c.id === exportId);

  return (
    <div className="modal-backdrop" onClick={closeExport}>
      <div className="modal export-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2>Экспорт{conv ? `: ${conv.title}` : ""}</h2>
          <button className="close-btn" onClick={closeExport} aria-label="Close">
            ✕
          </button>
        </div>
        <div className="modal-body export-choices">
          <button className="export-choice" onClick={() => exportPdf(exportId)}>
            <span className="export-choice-icon">📄</span>
            <span className="export-choice-name">PDF</span>
            <span className="export-choice-note">
              Через системную печать — выберите «Сохранить как PDF».
            </span>
          </button>
          <button className="export-choice" onClick={() => exportMarkdown(exportId)}>
            <span className="export-choice-icon">⤓</span>
            <span className="export-choice-name">Markdown</span>
            <span className="export-choice-note">Текстовый файл .md.</span>
          </button>
          <button className="export-choice" onClick={() => exportZip(exportId)}>
            <span className="export-choice-icon">🗜</span>
            <span className="export-choice-name">ZIP</span>
            <span className="export-choice-note">
              Транскрипт + аудио-записи в папку из настроек.
            </span>
          </button>
        </div>
      </div>
    </div>
  );
}
