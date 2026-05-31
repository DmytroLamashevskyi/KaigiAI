# KaigiAI (会議AI)

Десктоп-приложение для **реал-тайм двуязычного перевода живой речи**, работающее
**полностью офлайн**. Захватывает микрофон и системный звук, распознаёт речь и
переводит между двумя языками прямо на вашем устройстве — аудио и текст никуда не
уходят. Опционально можно подключить облачный API-провайдер для слабых ПК.

Стек: **Tauri v2 + Rust-ядро + React 18 / TypeScript (Vite)**. Интерфейс в стиле
Claude Desktop. Подробный дизайн — в [docs/PROJECT.md](docs/PROJECT.md).

## Возможности

- 🎙️ Захват микрофона + системного звука (WASAPI loopback) с VAD.
- 🗣️ **Локальное** распознавание речи (whisper.cpp) и автоопределение языка.
- 🔁 Перекрёстный перевод A→B / B→A двумя панелями параллельного транскрипта.
- 🧑‍🤝‍🧑 Диаризация спикеров (опц.) — метки `Speaker N` с персистентным
  переименованием на беседу (ONNX-эмбеддинг через `ort`).
- 📝 Заметки и AI-саммари беседы.
- 💾 История диалогов в локальном SQLite; экспорт txt / md / json.
- 🌐 14 языков интерфейса.
- ☁️ Опциональный API-режим (OpenAI-совместимый: облако / Ollama / LM Studio /
  корпоративный сервер) — для машин без GPU.

🔒 **Приватность.** Ключ API хранится в нативном хранилище секретов ОС (Windows
Credential Manager), **не** в открытом виде. Исходное аудио по умолчанию не
сохраняется — только текст.

## Быстрый старт (разработка)

Требуется Node.js 18+, Rust (stable) и MSVC build tools.

```powershell
npm install          # зависимости фронтенда
npm run dev          # Vite dev-сервер (только UI)
npm run tauri dev    # полное приложение (Rust + webview)
```

## Сборка релиза

```powershell
npm run tauri build
```

Артефакты появятся в `src-tauri/target/release/`:
- `app.exe` — портативный исполняемый файл;
- NSIS-инсталлятор и MSI — в `bundle/nsis/` и `bundle/msi/`.

## Локальные серверы и модели

Локальный режим поднимает whisper.cpp и llama.cpp как **сайдкары** (дочерние
процессы) и общается с ними по OpenAI-совместимому HTTP на `localhost` (порты 8771
для whisper, 8770 для llama, с фолбэком на свободный порт). Приложение само
запускает их перед записью, делает health-check и глушит при выходе.

Бинарники и веса **не входят в сборку** — их нужно скачать один раз и указать пути
в настройках. Ниже — протестированная конфигурация (CUDA 12.4, NVIDIA GPU).

### 1. Серверные бинарники (CUDA)

| Что | Источник |
| --- | --- |
| whisper.cpp (CUDA) | [whisper-cublas-12.4.0-bin-x64.zip](https://github.com/ggml-org/whisper.cpp/releases/download/v1.8.5/whisper-cublas-12.4.0-bin-x64.zip) (релиз `v1.8.5`) |
| llama.cpp (CUDA) | [llama-b9442-bin-win-cuda-12.4-x64.zip](https://github.com/ggml-org/llama.cpp/releases/download/b9442/llama-b9442-bin-win-cuda-12.4-x64.zip) (релиз `b9442`) |
| CUDA-рантайм для llama | [cudart-llama-bin-win-cuda-12.4-x64.zip](https://github.com/ggml-org/llama.cpp/releases/download/b9442/cudart-llama-bin-win-cuda-12.4-x64.zip) |

Распакуйте whisper-архив в отдельную папку (внутри будет `whisper-server.exe`).
llama-архив и `cudart`-архив распакуйте в **одну общую** папку (DLL из cudart
должны лежать рядом с `llama-server.exe`).

> Без GPU берите `*-bin-win-cpu-x64`-сборки с тех же релизов и поставьте «Слои на
> GPU» = 0.

### 2. Модели

| Роль | Файл | Источник |
| --- | --- | --- |
| STT (Whisper) | `ggml-medium.bin` (~1.5 ГБ) | [HuggingFace](https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin) |
| LLM (перевод) | `qwen2.5-3b-instruct-q5_k_m.gguf` (~2.3 ГБ) | [HuggingFace](https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q5_k_m.gguf) |
| Диаризация (опц.) | `wespeaker_en_voxceleb_resnet34_LM.onnx` (~25 МБ) | [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/wespeaker_en_voxceleb_resnet34_LM.onnx) |

> Для машин помощнее (16 ГБ VRAM) можно взять `large-v3` STT и Qwen2.5-7B Q5 —
> см. тиры железа в [docs/PROJECT.md](docs/PROJECT.md) §10.1.

### 3. Настройка в приложении

Откройте **Настройки → ИИ-провайдер → режим «💻 Локально»** и укажите:

- путь к `whisper-server.exe` и к модели Whisper (`.bin`);
- путь к `llama-server.exe` и к модели LLM (`.gguf`);
- **Слои на GPU** (`-ngl`): `0` — только CPU; больше нуля — выгрузка слоёв на GPU
  (на CUDA-сборке смело ставьте `99`, чтобы выгрузить всё, что влезает в VRAM);
- (опц.) путь к ONNX-модели диаризации — пустой путь выключает диаризацию.

После этого начните запись — приложение само поднимет серверы локально.

## Лицензия

MIT (см. [docs/PROJECT.md](docs/PROJECT.md) §4). Веса моделей не распространяются —
их скачивает пользователь; дефолтные модели выбраны с пермиссивной лицензией.
