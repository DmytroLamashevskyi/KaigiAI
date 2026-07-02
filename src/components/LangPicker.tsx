import { useState } from "react";

interface Props {
  /** Language codes offered in the menu, in display order. */
  options: string[];
  /** Label for an option (nativeName for the add-picker, languageName for ⇄). */
  renderLabel: (code: string) => string;
  onPick: (code: string) => void;
  /** Trigger button label. */
  label?: string;
  /** Trigger button tooltip. */
  title?: string;
  /** Optional heading rendered at the top of the menu. */
  header?: string;
  /** Wrapper class — "lang-reassign" anchors the ⇄ variant's hover-reveal CSS. */
  className?: string;
  triggerClassName?: string;
}

/** Dropdown of language codes behind a small trigger button. Two callers share
 *  it: the "＋ язык" control that grows a chat into the N-column grid, and the
 *  per-row ⇄ control that reassigns which language a message was spoken in
 *  (§10.7) — same backdrop/menu/item structure, different trigger and options. */
export default function LangPicker({
  options,
  renderLabel,
  onPick,
  label = "＋ язык",
  title = "Добавить язык в разговор",
  header,
  className = "lang-picker",
  triggerClassName = "lang-add-btn",
}: Props) {
  const [open, setOpen] = useState(false);
  if (options.length === 0) return null;

  return (
    <span className={className}>
      <button
        type="button"
        className={triggerClassName}
        onClick={() => setOpen((v) => !v)}
        title={title}
      >
        {label}
      </button>
      {open && (
        <>
          <div className="lang-picker-backdrop" onClick={() => setOpen(false)} />
          <div className="lang-picker-menu" role="menu">
            {header && <div className="lang-picker-label">{header}</div>}
            {options.map((code) => (
              <button
                key={code}
                type="button"
                className="lang-picker-item"
                onClick={() => {
                  onPick(code);
                  setOpen(false);
                }}
              >
                {renderLabel(code)}
              </button>
            ))}
          </div>
        </>
      )}
    </span>
  );
}
