// Latency diagnostics helpers (docs/PROJECT.md §10.16): the per-message hover
// breakdown and the per-conversation median / p95 summary in Settings.

import type { Message, Timings } from "../types";
import { languageName } from "../data/languages";

export const formatMs = (v: number): string => (v >= 1000 ? `${(v / 1000).toFixed(1)} с` : `${Math.round(v)} мс`);

/** Hover text for a message's time: wall-clock date plus, in diagnostics mode,
 *  the per-stage breakdown. */
export function timeTitle(createdAt: number, timings: Timings | null | undefined, diagnostics: boolean): string {
  const date = new Date(createdAt).toLocaleString();
  if (!diagnostics || !timings) return date;
  const lines = [
    date,
    `Ожидание паузы: ${formatMs(timings.hangoverMs)}`,
    timings.queueMs > 0 ? `Очередь: ${formatMs(timings.queueMs)}` : null,
    timings.segmentMs > 0 ? `Сегментация: ${formatMs(timings.segmentMs)}` : null,
    `Распознавание: ${formatMs(timings.sttMs)}`,
    ...Object.entries(timings.translateMs)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([lang, v]) => `Перевод → ${languageName(lang)}: ${formatMs(v)}`),
    `Обработка всего: ${formatMs(timings.totalMs)}`,
    `Конец речи → текст: ${formatMs(timings.hangoverMs + timings.totalMs)}`,
    `Длина фразы: ${formatMs(timings.audioMs)}`,
  ];
  return lines.filter(Boolean).join("\n");
}

export interface StageStat {
  label: string;
  median: number;
  p95: number;
}

/** Nearest-rank percentile of an ascending-sorted, non-empty array. */
function percentile(sorted: number[], p: number): number {
  const i = Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1));
  return sorted[i];
}

function stat(label: string, values: number[]): StageStat | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return { label, median: percentile(sorted, 50), p95: percentile(sorted, 95) };
}

/** Median / p95 per stage over the messages that carry timings. "Перевод (все
 *  языки)" sums a message's translations — today they run one after another. */
export function latencyStats(messages: Message[]): { count: number; stages: StageStat[] } {
  const ts = messages.map((m) => m.timings).filter((t): t is Timings => !!t);
  const pick = (f: (t: Timings) => number) => ts.map(f);
  const sumTr = (t: Timings) => Object.values(t.translateMs).reduce((a, b) => a + b, 0);
  const stages = [
    stat("Конец речи → текст", pick((t) => t.hangoverMs + t.totalMs)),
    stat("Ожидание паузы", pick((t) => t.hangoverMs)),
    stat("Очередь", pick((t) => t.queueMs)),
    stat("Сегментация", pick((t) => t.segmentMs)),
    stat("Распознавание", pick((t) => t.sttMs)),
    stat("Перевод (все языки)", pick(sumTr)),
    stat("Обработка всего", pick((t) => t.totalMs)),
  ].filter((s): s is StageStat => s !== null);
  return { count: ts.length, stages };
}
