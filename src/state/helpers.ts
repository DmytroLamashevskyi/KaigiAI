import type { Backend } from "../backend";
import { getBackend } from "../backend";
import type { Conversation, Message, ProviderMode, Settings } from "../types";
import { conversationLangs, displaySpeaker, textForLang } from "../types";
import { languageName } from "../data/languages";

/** A `.catch` handler that logs with a contextual prefix. */
export function logErr(context: string): (e: unknown) => void {
  return (e) => console.error(context, e);
}

/** User-facing message for an unknown thrown value (used for the error toast). */
export function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Translate `text` into every target language, resolving to a langCode→text
 *  map (§10.7). A failed target logs under `errContext` and is simply absent
 *  from the map, so one bad language never rejects the whole fan-out. Callers
 *  own the persistence ordering around this — see addTextMessage/setMessageLang. */
export function translateAll(
  backend: Pick<Backend, "translate">,
  text: string,
  from: string,
  targets: string[],
  errContext: string
): Promise<Record<string, string>> {
  return Promise.all(
    targets.map((to) =>
      backend
        .translate(text, from, to)
        .then((t) => [to, t] as const)
        .catch((e) => {
          logErr(errContext)(e);
          return [to, ""] as const;
        })
    )
  ).then((pairs) => {
    const translations: Record<string, string> = {};
    for (const [to, t] of pairs) if (t) translations[to] = t;
    return translations;
  });
}

/** Open (or reveal) a presentation window. Routes through the backend: a native
 *  Tauri window in the desktop app, or window.open in the browser (window.open
 *  doesn't work in the Tauri webview). `title` is the language name shown in
 *  the window caption. */
export function openPresentWindow(slot: "A" | "B", title: string): void {
  getBackend().openPresent(slot, title).catch(logErr("openPresent failed"));
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
    const next = { ...s, sttMode: legacy, translationMode: legacy };
    // Drop the obsolete key so it doesn't linger in the persisted settings blob.
    delete (next as { providerMode?: ProviderMode }).providerMode;
    return next;
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

/** Which conversation language typed text is in, by script — so manual input is
 *  placed in the right column and translated the right way (e.g. English typed
 *  into a RU↔EN chat lands on EN, not RU; §10.7). Snaps to the single language
 *  whose writing system matches; falls back to the first language when ambiguous. */
export function detectMessageLang(text: string, langs: string[]): string {
  const script = dominantScript(text);
  const fits = langs.filter((l) => scriptFitsLang(script, l));
  if (fits.length === 1) return fits[0];
  return langs[0] ?? "";
}

function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/** Markdown export of a conversation (used by the Export modal). Lists every
 *  conversation language (§10.7): the original line, then a quote per other
 *  language. A "foreign" original (spoken outside the language set) is tagged. */
export function conversationMarkdown(conv: Conversation, msgs: Message[]): string {
  const langs = conversationLangs(conv);
  const lines = [
    `# ${conv.title}`,
    langs.map(languageName).join(" ↔ "),
    "",
  ];
  for (const m of msgs) {
    const name = displaySpeaker(conv, m.speaker);
    const who = name ? `**${name}** ` : "";
    const foreign = !langs.includes(m.detectedLang);
    const tag = foreign ? `[${languageName(m.detectedLang)}] ` : "";
    lines.push(`\`${clockOf(m.createdAt)}\` ${who}${tag}${m.originalText}`);
    for (const lang of langs) {
      if (lang === m.detectedLang) continue;
      const t = textForLang(m, lang, conv);
      if (t) lines.push(`> ${t}`);
    }
    lines.push("");
  }
  return lines.join("\n");
}

/** A self-printing HTML page for PDF export via the system print dialog. */
export function conversationPrintHtml(conv: Conversation, msgs: Message[]): string {
  const convLangs = conversationLangs(conv);
  const rows = msgs
    .map((m) => {
      const name = displaySpeaker(conv, m.speaker);
      const foreign = !convLangs.includes(m.detectedLang);
      const meta = [clockOf(m.createdAt), name, foreign ? languageName(m.detectedLang) : ""]
        .filter((x): x is string => !!x)
        .map(escapeHtml)
        .join(" · ");
      const trans = convLangs
        .filter((lang) => lang !== m.detectedLang)
        .map((lang) => textForLang(m, lang, conv))
        .filter(Boolean)
        .map((t) => `<div class="t">${escapeHtml(t)}</div>`)
        .join("");
      return `<div class="m"><div class="meta">${meta}</div><div class="o">${escapeHtml(
        m.originalText
      )}</div>${trans}</div>`;
    })
    .join("");
  const title = escapeHtml(conv.title);
  const langs = convLangs.map((l) => escapeHtml(languageName(l))).join(" ↔ ");
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
