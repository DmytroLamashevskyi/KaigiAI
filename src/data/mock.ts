import type { Conversation, Message } from "../types";

const now = Date.now();

export const MOCK_CONVERSATIONS: Conversation[] = [
  {
    id: "c1",
    title: "Встреча с командой Токио",
    langA: "ru",
    langB: "ja",
    createdAt: now - 86400000,
    updatedAt: now - 3600000,
  },
  {
    id: "c2",
    title: "Onboarding call",
    langA: "en",
    langB: "ru",
    createdAt: now - 172800000,
    updatedAt: now - 172000000,
  },
];

export const MOCK_MESSAGES: Record<string, Message[]> = {
  c1: [
    {
      id: "m1",
      conversationId: "c1",
      source: "mic",
      detectedLang: "ru",
      speaker: "A",
      originalText: "Добрый день! Рад наконец познакомиться лично.",
      translatedText: "こんにちは！ようやく直接お会いできて嬉しいです。",
      startMs: 0,
      endMs: 3200,
      createdAt: now - 3600000,
    },
    {
      id: "m2",
      conversationId: "c1",
      source: "system",
      detectedLang: "ja",
      speaker: "B",
      originalText: "こちらこそ。プロジェクトの進捗を確認しましょう。",
      translatedText: "Взаимно. Давайте проверим прогресс по проекту.",
      startMs: 3500,
      endMs: 7000,
      createdAt: now - 3500000,
    },
    {
      id: "m3",
      conversationId: "c1",
      source: "mic",
      detectedLang: "ru",
      speaker: "A",
      originalText: "Конечно. Мы закончили первый этап на прошлой неделе.",
      translatedText: "もちろんです。先週、第一段階を完了しました。",
      startMs: 7200,
      endMs: 11000,
      createdAt: now - 3400000,
    },
  ],
  c2: [
    {
      id: "m4",
      conversationId: "c2",
      source: "mic",
      detectedLang: "en",
      speaker: "A",
      originalText: "Welcome aboard. Let me walk you through the setup.",
      translatedText: "Добро пожаловать. Давай проведу тебя по настройке.",
      startMs: 0,
      endMs: 3000,
      createdAt: now - 172000000,
    },
  ],
};
