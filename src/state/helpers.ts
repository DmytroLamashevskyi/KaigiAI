import type { Conversation, Message, ProviderMode, Settings } from "../types";
import { displaySpeaker } from "../types";
import { languageName } from "../data/languages";

/** A `.catch` handler that logs with a contextual prefix. */
export function logErr(context: string): (e: unknown) => void {
  return (e) => console.error(context, e);
}

/** Short random id for client-created conversations/messages. */
export function makeId(): string {
  return Math.random().toString(36).slice(2, 10);
}

/** Carry forward settings saved before STT/translation were split: a single
 *  `providerMode` becomes both `sttMode` and `translationMode`. */
export function migrateSettings(s: Partial<Settings>): Partial<Settings> {
  const legacy = (s as { providerMode?: ProviderMode }).providerMode;
  if (legacy && s.sttMode === undefined && s.translationMode === undefined) {
    return { ...s, sttMode: legacy, translationMode: legacy };
  }
  return s;
}

/** Wall-clock time of a message (local system time, HH:MM:SS). */
function clockOf(ms: number): string {
  return new Date(ms).toLocaleTimeString();
}

type Script = "cyrillic" | "japanese" | "hangul" | "arabic" | "latin" | "other";

/** Dominant writing system of a string (mirrors the Rust `dominant_script`). */
function dominantScript(text: string): Script {
  let cyr = 0, jp = 0, han = 0, ar = 0, lat = 0;
  for (const ch of text) {
    const c = ch.codePointAt(0)!;
    if (c >= 0x0400 && c <= 0x04ff) cyr++;
    else if (c >= 0x3040 && c <= 0x30ff) jp += 2; // kana — uniquely Japanese
    else if (c >= 0x4e00 && c <= 0x9fff) jp += 1; // kanji / CJK
    else if (c >= 0xac00 && c <= 0xd7af) han++;
    else if (c >= 0x0600 && c <= 0x06ff) ar++;
    else if ((c >= 65 && c <= 90) || (c >= 97 && c <= 122)) lat++;
  }
  const max = Math.max(cyr, jp, han, ar, lat);
  if (max === 0) return "other";
  if (max === cyr) return "cyrillic";
  if (max === jp) return "japanese";
  if (max === han) return "hangul";
  if (max === ar) return "arabic";
  return "latin";
}

function scriptFitsLang(script: Script, lang: string): boolean {
  const expected: Script =
    ["ru", "uk", "be", "bg", "sr", "mk"].includes(lang)
      ? "cyrillic"
      : lang === "ja" || lang === "zh"
        ? "japanese"
        : lang === "ko"
          ? "hangul"
          : ["ar", "fa", "ur"].includes(lang)
            ? "arabic"
            : "latin";
  return script === expected;
}

/** Which side ("A"/"B") typed text belongs to, by script — so manual input is
 *  placed in the right column and translated the right way (e.g. English typed
 *  into a RU↔EN chat goes to the EN side, not RU). Defaults to A when ambiguous. */
export function detectMessageSide(text: string, langA: string, langB: string): "A" | "B" {
  const script = dominantScript(text);
  const aFits = scriptFitsLang(script, langA);
  const bFits = scriptFitsLang(script, langB);
  if (aFits && !bFits) return "A";
  if (bFits && !aFits) return "B";
  return "A";
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/** Markdown export of a conversation (used by the Export modal). */
export function conversationMarkdown(conv: Conversation, msgs: Message[]): string {
  const lines = [
    `# ${conv.title}`,
    `${languageName(conv.langA)} ↔ ${languageName(conv.langB)}`,
    "",
  ];
  for (const m of msgs) {
    const name = displaySpeaker(conv, m.speaker);
    const who = name ? `**${name}** ` : "";
    const foreign =
      m.detectedLang !== conv.langA && m.detectedLang !== conv.langB;
    const tag = foreign ? `[${languageName(m.detectedLang)}] ` : "";
    lines.push(`\`${clockOf(m.createdAt)}\` ${who}${tag}${m.originalText}`);
    if (m.translatedText) lines.push(`> ${m.translatedText}`);
    if (foreign && m.translatedTextB) lines.push(`> ${m.translatedTextB}`);
    lines.push("");
  }
  return lines.join("\n");
}

/** A self-printing HTML page for PDF export via the system print dialog. */
export function conversationPrintHtml(conv: Conversation, msgs: Message[]): string {
  const rows = msgs
    .map((m) => {
      const name = displaySpeaker(conv, m.speaker);
      const foreign =
        m.detectedLang !== conv.langA && m.detectedLang !== conv.langB;
      const meta = [clockOf(m.createdAt), name, foreign ? languageName(m.detectedLang) : ""]
        .filter((x): x is string => !!x)
        .map(escapeHtml)
        .join(" · ");
      const trans = [m.translatedText, foreign ? m.translatedTextB : ""]
        .filter(Boolean)
        .map((t) => `<div class="t">${escapeHtml(t as string)}</div>`)
        .join("");
      return `<div class="m"><div class="meta">${meta}</div><div class="o">${escapeHtml(
        m.originalText
      )}</div>${trans}</div>`;
    })
    .join("");
  const title = escapeHtml(conv.title);
  const langs = `${escapeHtml(languageName(conv.langA))} ↔ ${escapeHtml(
    languageName(conv.langB)
  )}`;
  const date = escapeHtml(new Date().toLocaleString());
  return `<!doctype html><html><head><meta charset="utf-8"><title>${title}</title>
<style>
  body{font-family:"Segoe UI",system-ui,sans-serif;color:#1a1a1a;margin:32px;line-height:1.5}
  h1{font-size:20px;margin:0 0 2px}
  .sub{color:#666;font-size:12px;margin-bottom:20px}
  .m{margin:0 0 14px;padding-bottom:10px;border-bottom:1px solid #eee}
  .meta{font-size:11px;color:#888;margin-bottom:3px}
  .o{font-weight:600}
  .t{color:#555;margin-top:2px}
  @media print{.m{break-inside:avoid}}
</style></head><body>
<h1>${title}</h1><div class="sub">${langs} · ${date}</div>
${rows}
</body></html>`;
}
