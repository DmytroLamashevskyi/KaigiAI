import type { ReactNode } from "react";

interface Props {
  title: ReactNode;
  /** Modal variant class appended to `modal` (e.g. "export-modal"). */
  className: string;
  onClose: () => void;
  children: ReactNode;
}

/** Shared modal scaffold: dimmed backdrop (click closes), stopPropagation on the
 *  dialog itself, and the standard header with a ✕ button. Callers keep their
 *  own body/footer structure and visibility guards. */
export default function Modal({ title, className, onClose, children }: Props) {
  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className={`modal ${className}`} onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2>{title}</h2>
          <button className="close-btn" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}
