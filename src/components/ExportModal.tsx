import { useApp } from "../state/AppState";
import Modal from "./Modal";

/** Export picker: choose Markdown (downloads a .md file) or PDF (opens the
 *  system print dialog → "Save as PDF", which renders Japanese/Cyrillic with
 *  real system fonts). Opened from the conversation menu in the sidebar. */
export default function ExportModal() {
  const {
    exportId,
    closeExport,
    exportMarkdown,
    exportCsv,
    exportPdf,
    exportZip,
    conversations,
  } = useApp();
  if (!exportId) return null;
  const conv = conversations.find((c) => c.id === exportId);

  return (
    <Modal
      title={`Экспорт${conv ? `: ${conv.title}` : ""}`}
      className="export-modal"
      onClose={closeExport}
    >
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
        <button className="export-choice" onClick={() => exportCsv(exportId)}>
          <span className="export-choice-icon">📊</span>
          <span className="export-choice-name">CSV</span>
          <span className="export-choice-note">
            Таблица: реплика в строке, колонка на каждый язык. Открывается в Excel.
          </span>
        </button>
        <button className="export-choice" onClick={() => exportZip(exportId)}>
          <span className="export-choice-icon">🗜</span>
          <span className="export-choice-name">ZIP</span>
          <span className="export-choice-note">
            Транскрипт + аудио-записи в папку из настроек.
          </span>
        </button>
      </div>
    </Modal>
  );
}
