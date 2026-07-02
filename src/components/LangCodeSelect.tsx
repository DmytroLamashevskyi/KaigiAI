import { LANGUAGES } from "../data/languages";

interface Props {
  value: string;
  onChange: (code: string) => void;
  /** Codes rendered disabled (already taken by another column/side). */
  disabledCodes: string[];
  className?: string;
}

/** The transcript-header language `<select>` over the full LANGUAGES catalog,
 *  shared by the 2-pane A/B selectors and the N-column grid header (which were
 *  mirror copies of the same option loop). Settings' LangSelect stays separate —
 *  it renders different labels ("native (English)"). */
export default function LangCodeSelect({
  value,
  onChange,
  disabledCodes,
  className = "lang-select",
}: Props) {
  return (
    <select
      className={className}
      value={value}
      onChange={(e) => onChange(e.target.value)}
    >
      {LANGUAGES.map((l) => (
        <option
          key={l.code}
          value={l.code}
          disabled={disabledCodes.includes(l.code)}
        >
          {l.nativeName}
        </option>
      ))}
    </select>
  );
}
