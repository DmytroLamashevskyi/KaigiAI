import { useState } from "react";
import { LANGUAGES } from "../data/languages";

interface Props {
  /** Languages already chosen — excluded from the list (or disabled). */
  exclude: string[];
  /** Compact "+ язык" trigger label. */
  label?: string;
  onPick: (code: string) => void;
  disabled?: boolean;
}

/** A small "+ язык" button that opens a dropdown of languages not yet in the
 *  conversation, used to grow a 2-language chat into the N-column grid (§10.7). */
export default function LangPicker({ exclude, label = "＋ язык", onPick, disabled }: Props) {
  const [open, setOpen] = useState(false);
  const available = LANGUAGES.filter((l) => !exclude.includes(l.code));
  if (available.length === 0) return null;

  return (
    <span className="lang-picker">
      <button
        type="button"
        className="lang-add-btn"
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
        title="Добавить язык в разговор"
      >
        {label}
      </button>
      {open && (
        <>
          <div className="lang-picker-backdrop" onClick={() => setOpen(false)} />
          <div className="lang-picker-menu" role="menu">
            {available.map((l) => (
              <button
                key={l.code}
                type="button"
                className="lang-picker-item"
                onClick={() => {
                  onPick(l.code);
                  setOpen(false);
                }}
              >
                {l.nativeName}
              </button>
            ))}
          </div>
        </>
      )}
    </span>
  );
}
