//! Quality + latency benchmark over a FLEURS sample (docs/PROJECT.md §10.19).
//!
//! Runs pre-cut utterances through the SAME providers the app uses (settings,
//! sidecar servers, `SttProvider` / `TranslationProvider`), without the VAD, and
//! reports speech-recognition error rate, language-detection accuracy,
//! translation chrF against the FLEURS parallel references, and per-stage
//! latency. Data: `node scripts/fleurs-fetch.mjs`; CLI: `examples/bench.rs`.
//!
//! Sentences are translated in isolation (no conversation context), as the
//! first utterance of a conversation would be.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};

use crate::provider::{self, SttProvider, TranslationProvider};

/// The two providers a benchmark run drives.
pub type Providers = (Box<dyn SttProvider>, Box<dyn TranslationProvider>);
use crate::sidecar::Sidecars;

// ---------------------------------------------------------------------------
// Metrics (pure, unit-tested)
// ---------------------------------------------------------------------------

/// Lowercase, punctuation → space, collapse whitespace, ё → е (FLEURS
/// references spell ё, whisper often doesn't). Applied to both the reference
/// and the hypothesis before WER/CER.
pub fn normalize(text: &str) -> String {
    let mapped: String = text
        .chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'ё' => 'е',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Scored by CER rather than WER: scripts without spaces between words (ja,
/// zh), and Korean, whose spacing is inconsistent enough that WER mostly
/// measures spacing (CER is the standard metric for Korean ASR).
pub fn uses_cer(lang: &str) -> bool {
    matches!(lang, "ja" | "zh" | "ko")
}

/// Levenshtein distance between two token sequences.
fn edit_distance<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(x != y);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// (edits, reference length) for one utterance — WER over words, or CER over
/// non-space characters for [`uses_cer`] languages. Summed over the corpus.
pub fn error_counts(reference: &str, hypothesis: &str, lang: &str) -> (usize, usize) {
    let (r, h) = (normalize(reference), normalize(hypothesis));
    if uses_cer(lang) {
        let rc: Vec<char> = r.chars().filter(|c| !c.is_whitespace()).collect();
        let hc: Vec<char> = h.chars().filter(|c| !c.is_whitespace()).collect();
        (edit_distance(&rc, &hc), rc.len())
    } else {
        let rw: Vec<&str> = r.split_whitespace().collect();
        let hw: Vec<&str> = h.split_whitespace().collect();
        (edit_distance(&rw, &hw), rw.len())
    }
}

const CHRF_ORDER: usize = 6;
const CHRF_BETA: f64 = 2.0;

/// Corpus-level chrF statistics: per n-gram order, (matches, hyp n-grams, ref
/// n-grams). Whitespace is removed, as in sacrebleu's default chrF.
#[derive(Debug, Clone, Default)]
pub struct ChrfStats([(usize, usize, usize); CHRF_ORDER]);

impl ChrfStats {
    pub fn add(&mut self, hypothesis: &str, reference: &str) {
        let h: Vec<char> = hypothesis.chars().filter(|c| !c.is_whitespace()).collect();
        let r: Vec<char> = reference.chars().filter(|c| !c.is_whitespace()).collect();
        for n in 1..=CHRF_ORDER {
            let hg = ngrams(&h, n);
            let rg = ngrams(&r, n);
            let matches: usize = hg
                .iter()
                .map(|(g, c)| (*c).min(*rg.get(g).unwrap_or(&0)))
                .sum();
            let s = &mut self.0[n - 1];
            s.0 += matches;
            s.1 += hg.values().sum::<usize>();
            s.2 += rg.values().sum::<usize>();
        }
    }

    /// chrF score 0–100: precision and recall averaged over the n-gram orders
    /// that occur, then combined as F-beta (beta = 2, recall-weighted).
    pub fn score(&self) -> f64 {
        let orders: Vec<_> = self.0.iter().filter(|s| s.1 > 0 && s.2 > 0).collect();
        if orders.is_empty() {
            return 0.0;
        }
        let k = orders.len() as f64;
        let p = orders.iter().map(|s| s.0 as f64 / s.1 as f64).sum::<f64>() / k;
        let r = orders.iter().map(|s| s.0 as f64 / s.2 as f64).sum::<f64>() / k;
        if p + r == 0.0 {
            return 0.0;
        }
        let b2 = CHRF_BETA * CHRF_BETA;
        100.0 * (1.0 + b2) * p * r / (b2 * p + r)
    }
}

fn ngrams(chars: &[char], n: usize) -> HashMap<&[char], usize> {
    let mut m = HashMap::new();
    if chars.len() >= n {
        for w in chars.windows(n) {
            *m.entry(w).or_insert(0) += 1;
        }
    }
    m
}

/// Nearest-rank percentile (0–100) of unsorted values; 0 for an empty set.
pub fn percentile(values: &[i64], p: f64) -> i64 {
    if values.is_empty() {
        return 0;
    }
    let mut v = values.to_vec();
    v.sort_unstable();
    let rank = ((p / 100.0) * v.len() as f64).ceil() as usize;
    v[rank.clamp(1, v.len()) - 1]
}

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

/// `bench/data/manifest.json` written by `scripts/fleurs-fetch.mjs`.
#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub langs: Vec<String>,
    pub items: HashMap<String, Vec<Item>>,
    /// sentence id → lang → reference text.
    pub refs: HashMap<String, HashMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    pub id: String,
    pub file: String,
}

pub fn load_manifest(data_dir: &Path) -> Result<Manifest, String> {
    let path = data_dir.join("manifest.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("{}: {e} — run `node scripts/fleurs-fetch.mjs` first", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("{}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TranslateMode {
    /// One target language after another — what the app does today.
    Sequential,
    /// All target languages concurrently.
    Parallel,
    /// Both, per utterance (quality is taken from the sequential run).
    Both,
}

pub struct BenchOpts {
    pub data_dir: PathBuf,
    /// Source languages to run (default: every language in the manifest).
    pub sources: Option<Vec<String>>,
    /// Target languages for translation (default: every language in the manifest).
    pub targets: Option<Vec<String>>,
    pub limit: usize,
    pub mode: TranslateMode,
    /// App settings blob (already loaded and overridden by the CLI).
    pub settings: serde_json::Value,
    /// Free-form label for the report (e.g. "rtx3080ti-medium-3b").
    pub label: String,
}

/// One utterance's raw result (also written to the JSON report).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemResult {
    pub lang: String,
    pub id: String,
    pub audio_ms: i64,
    pub stt_ms: i64,
    pub detected_lang: String,
    pub reference: String,
    pub transcript: String,
    pub edits: usize,
    pub ref_len: usize,
    /// target → (translation, ms) from the sequential run (or the parallel
    /// run in parallel-only mode).
    pub translations: BTreeMap<String, (String, i64)>,
    /// Sum of per-target times when run one after another.
    pub seq_total_ms: Option<i64>,
    /// Wall time of all targets requested concurrently.
    pub par_total_ms: Option<i64>,
    /// Provider errors for this utterance (STT / per-target translation).
    pub errors: Vec<String>,
}

pub struct BenchReport {
    pub markdown: String,
    pub items: Vec<ItemResult>,
}

/// The desktop app's database: `%APPDATA%i.kaigi.desktop\kaigi.db` on Windows
/// (Tauri's app data dir for the bundle identifier).
pub fn default_db_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("ai.kaigi.desktop").join("kaigi.db"))
}

/// The app's saved settings plus the API key from the OS keychain — so the
/// benchmark runs exactly the configuration the user runs.
pub async fn load_app_settings(db_path: &Path) -> Result<serde_json::Value, String> {
    if !db_path.exists() {
        return Err(format!("{} not found — start the app once, or pass --settings", db_path.display()));
    }
    let db = crate::db::Db::connect(db_path).await.map_err(|e| e.to_string())?;
    let mut settings = db.get_app_settings().await.map_err(|e| e.to_string())?;
    if let (Some(key), Some(o)) = (crate::keychain::get_api_key(), settings.as_object_mut()) {
        o.insert("apiKey".into(), serde_json::Value::String(key));
    }
    Ok(settings)
}

/// Start the sidecar servers the settings need (exactly like `start_recording`)
/// and build the providers. Refuses to fall back to the mock provider — a
/// benchmark of mock output would be meaningless.
pub fn build_providers(
    settings: &mut serde_json::Value,
    sidecars: &Sidecars,
) -> Result<Providers, String> {
    let set = |s: &mut serde_json::Value, k: &str, v: String| {
        if let Some(o) = s.as_object_mut() {
            o.insert(k.into(), serde_json::Value::String(v));
        }
    };
    if provider::stt_is_local(settings) {
        let url = sidecars.ensure_whisper(settings)?;
        set(settings, "localSttBaseUrl", url);
    } else if !provider::stt_is_api(settings) {
        return Err("speech recognition is neither local nor a configured API (base URL + key)".into());
    }
    if provider::translation_is_local(settings) {
        let url = sidecars.ensure_llama(settings)?;
        set(settings, "localLlmBaseUrl", url);
    } else if !provider::translation_is_api(settings) {
        return Err("translation is neither local nor a configured API (base URL + key)".into());
    }
    Ok((
        provider::stt_from_settings(settings),
        provider::translation_from_settings(settings),
    ))
}

pub async fn run(
    opts: &BenchOpts,
    stt: &dyn SttProvider,
    translator: &dyn TranslationProvider,
) -> Result<BenchReport, String> {
    let manifest = load_manifest(&opts.data_dir)?;
    let sources = opts.sources.clone().unwrap_or_else(|| manifest.langs.clone());
    let targets = opts.targets.clone().unwrap_or_else(|| manifest.langs.clone());
    let mut items = Vec::new();

    // Warm-up, not measured: the first request to a fresh local server pays
    // CUDA/kernel initialization (~15–20 s for whisper), which would otherwise
    // land in the first utterance's latency.
    if let Some(first) = sources.first().and_then(|s| manifest.items.get(s)).and_then(|l| l.first()) {
        let bytes = std::fs::read(opts.data_dir.join(&first.file)).map_err(|e| e.to_string())?;
        let (pcm, rate) = crate::audio::decode_wav_mono(&bytes)?;
        let src = &sources[0];
        let t = stt.transcribe(&pcm, rate, std::slice::from_ref(src)).await?;
        if let Some(tgt) = targets.iter().find(|t| *t != src) {
            translator.translate(&t.text, src, tgt, "").await?;
        }
    }

    for src in &sources {
        let list = manifest
            .items
            .get(src)
            .ok_or_else(|| format!("no '{src}' utterances in the manifest"))?;
        let tgts: Vec<&String> = targets.iter().filter(|t| *t != src).collect();
        // Mirrors a conversation in these languages: whisper's hint list.
        let hint: Vec<String> = std::iter::once(src.clone())
            .chain(tgts.iter().map(|t| (*t).clone()))
            .collect();
        for (i, item) in list.iter().take(opts.limit).enumerate() {
            let refs = manifest.refs.get(&item.id).ok_or("manifest: missing refs")?;
            let reference = refs.get(src).cloned().unwrap_or_default();
            let bytes = std::fs::read(opts.data_dir.join(&item.file))
                .map_err(|e| format!("{}: {e}", item.file))?;
            let (pcm, rate) = crate::audio::decode_wav_mono(&bytes)?;
            if rate != crate::audio::SAMPLE_RATE {
                return Err(format!("{}: {rate} Hz, expected 16 kHz", item.file));
            }
            let audio_ms = (pcm.len() as f64 / rate as f64 * 1000.0) as i64;

            // A provider error is recorded and the run goes on, as in the app
            // (toast + empty cell) — one bad response must not void a 15-minute run.
            let mut errors = Vec::new();
            let started = Instant::now();
            let transcript = stt.transcribe(&pcm, rate, &hint).await.unwrap_or_else(|e| {
                errors.push(format!("stt: {e}"));
                provider::Transcript { text: String::new(), lang: String::new() }
            });
            let stt_ms = started.elapsed().as_millis() as i64;
            let (edits, ref_len) = error_counts(&reference, &transcript.text, src);

            let mut translations = BTreeMap::new();
            let (mut seq_total_ms, mut par_total_ms) = (None, None);
            if opts.mode != TranslateMode::Parallel {
                let mut total = 0;
                for t in &tgts {
                    let started = Instant::now();
                    let out = translator
                        .translate(&transcript.text, src, t, "")
                        .await
                        .unwrap_or_else(|e| {
                            errors.push(format!("seq {t}: {e}"));
                            String::new()
                        });
                    let ms = started.elapsed().as_millis() as i64;
                    total += ms;
                    translations.insert((*t).clone(), (out, ms));
                }
                seq_total_ms = Some(total);
            }
            if opts.mode != TranslateMode::Sequential {
                let started = Instant::now();
                let outs = join_all(tgts.iter().map(|t| {
                    let text = &transcript.text;
                    async move {
                        let s = Instant::now();
                        let r = translator.translate(text, src, t, "").await;
                        (r, s.elapsed().as_millis() as i64)
                    }
                }))
                .await;
                par_total_ms = Some(started.elapsed().as_millis() as i64);
                for (t, (r, ms)) in tgts.iter().zip(outs) {
                    match r {
                        Ok(out) if opts.mode == TranslateMode::Parallel => {
                            translations.insert((*t).clone(), (out, ms));
                        }
                        Ok(_) => {}
                        Err(e) => {
                            errors.push(format!("par {t}: {e}"));
                            if opts.mode == TranslateMode::Parallel {
                                translations.insert((*t).clone(), (String::new(), ms));
                            }
                        }
                    }
                }
            }
            for e in &errors {
                eprintln!("  provider error: {}", e.chars().take(200).collect::<String>());
            }
            eprintln!(
                "[{src} {}/{}] stt {stt_ms} ms, {}/{} edits{}{}",
                i + 1,
                list.len().min(opts.limit),
                edits,
                ref_len,
                seq_total_ms.map(|v| format!(", seq {v} ms")).unwrap_or_default(),
                par_total_ms.map(|v| format!(", par {v} ms")).unwrap_or_default(),
            );
            items.push(ItemResult {
                lang: src.clone(),
                id: item.id.clone(),
                audio_ms,
                stt_ms,
                detected_lang: transcript.lang,
                reference,
                transcript: transcript.text,
                edits,
                ref_len,
                translations,
                seq_total_ms,
                par_total_ms,
                errors,
            });
        }
    }

    let markdown = render(opts, &sources, &targets, &manifest, &items);
    Ok(BenchReport { markdown, items })
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// "local · ggml-medium.bin" / "API api.groq.com · whisper-large-v3".
fn describe_stage(settings: &serde_json::Value, stage: &str) -> String {
    let s = |k: &str| settings.get(k).and_then(|v| v.as_str()).unwrap_or("");
    let local = if stage == "stt" {
        provider::stt_is_local(settings)
    } else {
        provider::translation_is_local(settings)
    };
    if local {
        let path = if stage == "stt" { s("localWhisperPath") } else { s("localLlmPath") };
        format!("local · {}", file_name(path))
    } else {
        let host = s("apiBaseUrl")
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or("");
        let model = if stage == "stt" { s("sttModel") } else { s("llmModel") };
        format!("API {host} · {model}")
    }
}

/// GPU name(s) via nvidia-smi when available, plus the CPU identifier.
pub fn hardware() -> String {
    let gpu = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name,memory.total", "--format=csv,noheader"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().replace('\n', "; "))
        .filter(|s| !s.is_empty());
    let cpu = std::env::var("PROCESSOR_IDENTIFIER").ok();
    match (gpu, cpu) {
        (Some(g), Some(c)) => format!("{g} / {c}"),
        (Some(g), None) => g,
        (None, Some(c)) => format!("CPU only: {c}"),
        (None, None) => "unknown".into(),
    }
}

fn fmt_ms(ms: i64) -> String {
    if ms >= 1000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        format!("{ms} ms")
    }
}

fn render(
    opts: &BenchOpts,
    sources: &[String],
    targets: &[String],
    manifest: &Manifest,
    items: &[ItemResult],
) -> String {
    let mut md = String::new();
    let mut line = |s: String| {
        md.push_str(&s);
        md.push('\n');
    };
    line(format!("## KaigiAI benchmark — {}", opts.label));
    line(String::new());
    line(format!("- STT: {}", describe_stage(&opts.settings, "stt")));
    line(format!("- Translation: {}", describe_stage(&opts.settings, "translation")));
    line(format!("- Hardware: {}", hardware()));
    line(format!(
        "- Data: Google FLEURS dev (CC-BY-4.0), up to {} utterances per language, read speech, \
         sentences translated in isolation",
        opts.limit
    ));
    line(String::new());

    line("### Speech recognition".into());
    line(String::new());
    line("| Lang | N | Language detected | WER / CER | STT median | STT p95 | STT / audio |".into());
    line("| --- | --- | --- | --- | --- | --- | --- |".into());
    for src in sources {
        let rows: Vec<&ItemResult> = items.iter().filter(|r| &r.lang == src).collect();
        if rows.is_empty() {
            continue;
        }
        let n = rows.len();
        let detected = rows.iter().filter(|r| &r.detected_lang == src).count();
        let (e, l) = rows.iter().fold((0, 0), |(e, l), r| (e + r.edits, l + r.ref_len));
        let stt: Vec<i64> = rows.iter().map(|r| r.stt_ms).collect();
        let rtf: Vec<i64> = rows.iter().map(|r| r.stt_ms * 1000 / r.audio_ms.max(1)).collect();
        line(format!(
            "| {src} | {n} | {:.0}% | {:.1}% {} | {} | {} | {:.2} |",
            100.0 * detected as f64 / n as f64,
            100.0 * e as f64 / l.max(1) as f64,
            if uses_cer(src) { "CER" } else { "WER" },
            fmt_ms(percentile(&stt, 50.0)),
            fmt_ms(percentile(&stt, 95.0)),
            percentile(&rtf, 50.0) as f64 / 1000.0,
        ));
    }
    line(String::new());

    line("### Translation quality — chrF (recognized text → FLEURS reference)".into());
    line(String::new());
    line(format!("| Source \\ target | {} |", targets.join(" | ")));
    line(format!("| --- |{}", " --- |".repeat(targets.len())));
    for src in sources {
        let rows: Vec<&ItemResult> = items.iter().filter(|r| &r.lang == src).collect();
        if rows.is_empty() {
            continue;
        }
        let cells: Vec<String> = targets
            .iter()
            .map(|t| {
                if t == src {
                    return "—".into();
                }
                let mut stats = ChrfStats::default();
                for r in &rows {
                    let reference = manifest.refs.get(&r.id).and_then(|m| m.get(t));
                    if let (Some((hyp, _)), Some(reference)) = (r.translations.get(t), reference) {
                        stats.add(hyp, reference);
                    }
                }
                format!("{:.1}", stats.score())
            })
            .collect();
        line(format!("| **{src}** | {} |", cells.join(" | ")));
    }
    line(String::new());

    line("### Translation latency (all target languages of one utterance)".into());
    line(String::new());
    line("| Source | Targets | One translation, median | Sequential, median | Sequential, p95 | Parallel, median | Parallel, p95 |".into());
    line("| --- | --- | --- | --- | --- | --- | --- |".into());
    for src in sources {
        let rows: Vec<&ItemResult> = items.iter().filter(|r| &r.lang == src).collect();
        if rows.is_empty() {
            continue;
        }
        let one: Vec<i64> = rows
            .iter()
            .flat_map(|r| r.translations.values().map(|(_, ms)| *ms))
            .collect();
        let seq: Vec<i64> = rows.iter().filter_map(|r| r.seq_total_ms).collect();
        let par: Vec<i64> = rows.iter().filter_map(|r| r.par_total_ms).collect();
        let cell = |v: &[i64], p: f64| if v.is_empty() { "—".into() } else { fmt_ms(percentile(v, p)) };
        line(format!(
            "| {src} | {} | {} | {} | {} | {} | {} |",
            targets.iter().filter(|t| *t != src).count(),
            cell(&one, 50.0),
            cell(&seq, 50.0),
            cell(&seq, 95.0),
            cell(&par, 50.0),
            cell(&par, 95.0),
        ));
    }
    let errors: usize = items.iter().map(|r| r.errors.len()).sum();
    if errors > 0 {
        line(String::new());
        line(format!(
            "Provider errors: {errors} (failed requests count as empty output in the \
             scores; details in the JSON report)."
        ));
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_punctuation_and_case() {
        assert_eq!(normalize("Привет, мир!  Как  дела?"), "привет мир как дела");
        assert_eq!(normalize("東京へ行きます。"), "東京へ行きます");
        assert_eq!(normalize("Четвёрок"), "четверок");
    }

    #[test]
    fn wer_counts_word_edits() {
        // one substitution + one deletion over 4 reference words
        assert_eq!(error_counts("the cat sat down", "the dog sat", "en"), (2, 4));
        assert_eq!(error_counts("Hello, world.", "hello world", "en"), (0, 2));
    }

    #[test]
    fn cer_for_unspaced_scripts() {
        // 5 reference chars, one substituted
        assert_eq!(error_counts("東京へ行く", "東京に行く", "ja"), (1, 5));
    }

    #[test]
    fn chrf_bounds() {
        let mut same = ChrfStats::default();
        same.add("The quick brown fox", "The quick brown fox");
        assert!((same.score() - 100.0).abs() < 1e-9);

        let mut none = ChrfStats::default();
        none.add("xyz", "abc");
        assert_eq!(none.score(), 0.0);

        let mut partial = ChrfStats::default();
        partial.add("The quick brown dog", "The quick brown fox");
        let s = partial.score();
        assert!(s > 50.0 && s < 100.0, "{s}");
    }

    #[test]
    fn percentile_nearest_rank() {
        let v = [5, 1, 4, 2, 3];
        assert_eq!(percentile(&v, 50.0), 3);
        assert_eq!(percentile(&v, 95.0), 5);
        assert_eq!(percentile(&[], 50.0), 0);
    }
}
