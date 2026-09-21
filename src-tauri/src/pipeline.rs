//! Recording is owned by the controller; one bounded worker owns inference.
//! Only the controller can commit a complete result to a verified target.
use crate::{asr, audio, command, context, dictionary, formatter, hotkey, inject, runtime, store};
use audio::capture::{AudioClip, MicCapture};
use context::target::TargetSnapshot;
use formatter::{ContextEnvelope, FieldContext, Options};
use hotkey::TriggerEvent;
use runtime::{Command, LastResult};
use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};
use store::settings::Settings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordingMode {
    Hold,
    Toggle,
}
#[derive(Default)]
struct Controller {
    recording: Option<RecordingMode>,
    generation: u64,
    outstanding: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Start(RecordingMode),
    Finish,
    Cancel,
    None,
}
impl Controller {
    fn event(&mut self, event: &TriggerEvent, capacity: usize) -> Action {
        match event {
            TriggerEvent::Cancel => {
                self.generation = self.generation.wrapping_add(1);
                Action::Cancel
            }
            TriggerEvent::PttEnd if self.recording == Some(RecordingMode::Hold) => Action::Finish,
            TriggerEvent::Toggle | TriggerEvent::ManualToggle if self.recording.is_some() => {
                Action::Finish
            }
            TriggerEvent::Toggle | TriggerEvent::ManualToggle if self.outstanding < capacity => {
                Action::Start(RecordingMode::Toggle)
            }
            TriggerEvent::PttStart if self.recording.is_none() && self.outstanding < capacity => {
                Action::Start(RecordingMode::Hold)
            }
            _ => Action::None,
        }
    }
}
struct Active {
    settings: Settings,
    entries: Vec<dictionary::Entry>,
    target: TargetSnapshot,
    app: Option<String>,
    started: Instant,
}
struct Job {
    id: u64,
    generation: u64,
    clip: Arc<AudioClip>,
    settings: Settings,
    entries: Vec<dictionary::Entry>,
    target: Option<TargetSnapshot>,
    app: Option<String>,
    queued_at: Instant,
    finalize_ms: u128,
}
struct Completed {
    job: Job,
    result: LastResult,
}
fn process(job: &Job, generation: &AtomicU64) -> LastResult {
    let mut result = LastResult {
        id: job.id,
        insertion: "pending".into(),
        backend: job.settings.asr_backend.clone(),
        ..Default::default()
    };
    let mut timings = serde_json::json!({"queue":job.queued_at.elapsed().as_millis(),"finalization":job.finalize_ms});
    let total = Instant::now();
    let outcome = (|| -> Result<(), String> {
        if generation.load(Ordering::Acquire) != job.generation {
            return Err("cancelled".into());
        }
        runtime::update(|s| s.processing = true);
        let t = Instant::now();
        let pcm = job.clip.to_pcm16_16k();
        timings["resample"] = serde_json::json!(t.elapsed().as_millis());
        // Keep short/quiet utterances. Only exact digital silence is rejected.
        if pcm.is_empty() || pcm.iter().all(|&s| s == 0) {
            return Err("No microphone audio was captured.".into());
        }
        let t = Instant::now();
        let engine = asr::ensure_engine_for(&job.settings, job.settings.asr_backend_kind())?;
        timings["backend_ready"] = serde_json::json!(t.elapsed().as_millis());
        let vocabulary = dictionary::apply::snapshot_from_entries(&job.entries);
        let t = Instant::now();
        let raw = engine.transcribe(&pcm, "en", &vocabulary.canonical)?;
        timings["asr"] = serde_json::json!(t.elapsed().as_millis());
        if generation.load(Ordering::Acquire) != job.generation {
            return Err("cancelled".into());
        }
        result.raw = Some(raw.text.clone());
        let normalized = dictionary::apply::apply_snapshot(&raw.text, &vocabulary);
        result.normalized = Some(normalized.clone());
        let t = Instant::now();
        let normalized = if job.settings.stutter_correction {
            formatter::speech::prepare(&normalized, &vocabulary.canonical, true, true, true)
        } else {
            normalized
        };
        timings["speech_cleanup"] = serde_json::json!(t.elapsed().as_millis());
        result.final_text = Some(normalized.clone());
        if normalized.trim().is_empty() {
            result.insertion = "empty".into();
            return Ok(());
        }
        // Numbers work everywhere; browsers keep prose and writing apps get
        // automatic lists. Retain raw/normalized words for original recovery.
        if let Some(text) = formatter::layout::automatic_text(&normalized, job.app.as_deref()) {
            result.final_text = Some(text);
            return Ok(());
        }
        let category = job
            .app
            .as_deref()
            .map(context::category_for_exe)
            .unwrap_or("other");
        if job.settings.cleanup_mode == "polished"
            && category != "code"
            && command::classify(&normalized).is_none()
        {
            // Terminal TextPattern contains scrollback/status output, not the
            // draft being edited; it must not influence wording or casing.
            let ctx = job
                .target
                .as_ref()
                .filter(|t| !t.context.terminal)
                .map(|t| &t.context);
            let envelope = ContextEnvelope {
                mode: "dictation".into(),
                raw_transcript: normalized.clone(),
                context: FieldContext {
                    app: job.app.clone(),
                    app_category: category.into(),
                    text_before: ctx.and_then(|c| c.text_before.clone()),
                    selected_text: ctx.and_then(|c| c.selected_text.clone()),
                    text_after: ctx.and_then(|c| c.text_after.clone()),
                },
                user_style: Some(
                    serde_json::json!({"speech_cleanup":job.settings.stutter_correction}),
                ),
                language: "en-US".into(),
                vocabulary: Some(vocabulary.canonical),
                options: Options {
                    max_tokens: 256,
                    stream: false,
                },
            };
            let mut fmt = formatter::local_llm::OllamaFormatter::new(
                job.settings.formatter_port,
                &job.settings.formatter_model,
            );
            fmt.num_ctx = job.settings.formatter_num_ctx;
            let t = Instant::now();
            match formatter::format_complete(&fmt, &envelope) {
                Ok(candidate) => {
                    timings["formatter_load_ns"] =
                        serde_json::json!(candidate.metadata.load_duration_ns);
                    timings["formatter_eval_ns"] =
                        serde_json::json!(candidate.metadata.eval_duration_ns);
                    result.final_text = Some(candidate.text);
                }
                Err(error) => {
                    result.error = Some(format!(
                        "Polishing skipped; original wording preserved: {error}"
                    ));
                }
            }
            timings["format"] = serde_json::json!(t.elapsed().as_millis());
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        result.error = Some(error);
    }
    if let Some(text) = &mut result.final_text {
        *text = formatter::layout::for_app(text, job.app.as_deref()).into_owned();
    }
    timings["processing_total"] = serde_json::json!(total.elapsed().as_millis());
    timings["release_to_result"] =
        serde_json::json!(job.queued_at.elapsed().as_millis() + job.finalize_ms);
    result.stage_ms = timings;
    runtime::update(|s| s.processing = false);
    result
}

/// Explicit CLI/evaluation entry point; shares production processing and
/// never captures a microphone or commits text to an application.
pub fn replay_clip(
    clip: AudioClip,
    settings: Settings,
    entries: Vec<dictionary::Entry>,
) -> LastResult {
    let job = Job {
        id: 1,
        generation: 0,
        clip: Arc::new(clip),
        settings,
        entries,
        target: None,
        app: None,
        queued_at: Instant::now(),
        finalize_ms: 0,
    };
    process(&job, &AtomicU64::new(0))
}
fn chime(settings: &Settings, kind: audio::beep::ChimeKind) {
    if settings.chimes_enabled {
        audio::beep::play(kind);
    }
}
fn error(message: impl Into<String>) {
    runtime::update(|s| s.error = Some(message.into()));
}
fn finish(
    mic: &MicCapture,
    active: Active,
    generation: u64,
    id: u64,
    endpoint: Instant,
) -> Result<Job, String> {
    let t = Instant::now();
    let clip = mic.stop_at(
        endpoint,
        Duration::from_millis(active.settings.capture_drain_ms),
    )?;
    chime(&active.settings, audio::beep::ChimeKind::Done);
    Ok(Job {
        id,
        generation,
        clip: Arc::new(clip),
        settings: active.settings,
        entries: active.entries,
        target: (active.target.hwnd != 0).then_some(active.target),
        app: active.app,
        queued_at: Instant::now(),
        finalize_ms: t.elapsed().as_millis(),
    })
}
pub fn run_loop(mic: &mut MicCapture, dict: &dictionary::Dictionary) -> Result<(), String> {
    let settings = Settings::load();
    let capacity = settings.max_pending_utterances.clamp(1, 2) + 1;
    let (tx, rx) = mpsc::sync_channel::<Job>(capacity);
    let (result_tx, result_rx) = mpsc::channel::<Completed>();
    let generation = Arc::new(AtomicU64::new(0));
    let worker_generation = generation.clone();
    std::thread::Builder::new()
        .name("parla-inference".into())
        .spawn(move || {
            while let Ok(job) = rx.recv() {
                let result = process(&job, &worker_generation);
                if result_tx.send(Completed { job, result }).is_err() {
                    break;
                }
            }
        })
        .map_err(|e| format!("Cannot start inference worker: {e}"))?;
    let mut controller = Controller::default();
    let mut active: Option<Active> = None;
    let mut ready = VecDeque::<Completed>::new();
    let mut ledger = VecDeque::<inject::transaction::CommitReceipt>::new();
    let mut session = command::Session::new();
    let mut last_original = String::new();
    let mut restore_deadline: Option<Instant> = None;
    let mut history = store::history::LazyHistory::new();
    history.maintenance();
    let mut retention_checked = Instant::now();
    let mut next_id = 1u64;
    runtime::update(|s| {
        s.mode = "idle".into();
        s.device = mic.device_info();
    });
    loop {
        runtime::expire_audio();
        let mut events: Vec<_> = hotkey::poll_timed_events()
            .into_iter()
            .map(|(e, t)| (Some(e), t))
            .collect();
        if retention_checked.elapsed() >= Duration::from_secs(60) {
            history.maintenance();
            retention_checked = Instant::now();
        }
        while let Some(control) = runtime::take_command() {
            match control {
                Command::Stop if active.is_some() => events.push((None, Instant::now())),
                Command::Cancel | Command::Purge => {
                    events.push((Some(TriggerEvent::Cancel), Instant::now()));
                    runtime::purge();
                    session.forget();
                }
                Command::Retry if active.is_none() && controller.outstanding < capacity => {
                    if let Some(replay) = runtime::replay() {
                        let job = Job {
                            id: next_id,
                            generation: controller.generation,
                            clip: replay.clip,
                            settings: replay.settings,
                            entries: replay.entries,
                            target: None,
                            app: None,
                            queued_at: Instant::now(),
                            finalize_ms: 0,
                        };
                        next_id += 1;
                        if tx.try_send(job).is_ok() {
                            controller.outstanding += 1;
                        } else {
                            error("Inference queue is full.");
                        }
                    } else {
                        error("Retry audio expired or was not enabled.");
                    }
                }
                Command::Restore => {
                    if active.is_none()
                        && controller.outstanding == 0
                        && session.restore_target().is_some()
                    {
                        restore_deadline = Some(Instant::now() + Duration::from_secs(10));
                        error("Return to the original field within 10 seconds to restore the raw wording.");
                    } else {
                        error("No idle, verified insertion to restore; use Copy raw.");
                    }
                }
                Command::Learn { alias, canonical } => {
                    if alias.trim().is_empty()
                        || canonical.trim().is_empty()
                        || alias.len() > 200
                        || canonical.len() > 200
                    {
                        error("Use a short, nonempty correction.");
                    } else if let Err(e) = dict.add_entry(&dictionary::Entry {
                        term: alias.trim().into(),
                        replacement: Some(canonical.trim().into()),
                        snippet: None,
                    }) {
                        error(e);
                    }
                }
                _ => error("That action is unavailable while recording or the queue is full."),
            }
        }
        if let Some(current) = &active {
            if !mic.is_recording()
                || current.started.elapsed()
                    >= Duration::from_secs(current.settings.max_recording_seconds as u64)
            {
                error(mic.last_error().unwrap_or_else(|| {
                    "Recording reached its duration limit; transcribing the captured audio.".into()
                }));
                events.push((None, Instant::now()));
            }
        }
        for (event, at) in events {
            let action = match &event {
                Some(event) => controller.event(event, capacity),
                None if active.is_some() => Action::Finish,
                None => Action::None,
            };
            match action {
                Action::Start(mode) => {
                    restore_deadline = None;
                    let settings = Settings::load();
                    asr::set_backend(settings.asr_backend_kind());
                    let entries = dict.all_entries();
                    if mic.last_error().is_some() {
                        match MicCapture::open_with_settings(&settings) {
                            Ok(reopened) => *mic = reopened,
                            Err(e) => {
                                error(e);
                                continue;
                            }
                        }
                    }
                    if let Err(e) = mic.start() {
                        error(e);
                        continue;
                    }
                    let started = Instant::now();
                    let manual = matches!(event, Some(TriggerEvent::ManualToggle));
                    let mut target = TargetSnapshot::capture(120);
                    if target.context.password || (cfg!(windows) && target.hwnd == 0 && !manual) {
                        let _ = mic.stop_at(Instant::now(), Duration::ZERO);
                        error("Dictation cannot start in this field.");
                        continue;
                    }
                    let app = context::native::exe_for_window(target.hwnd);
                    // A dashboard or CLI trigger does not identify the user's
                    // intended destination. Record normally, then offer Copy.
                    if manual {
                        target.hwnd = 0;
                    }
                    controller.recording = Some(mode);
                    runtime::update(|s| {
                        s.recording = true;
                        s.recording_started = Some(started);
                        s.mode = if mode == RecordingMode::Toggle {
                            "toggle"
                        } else {
                            "hold"
                        }
                        .into();
                        s.settings = settings.clone();
                        s.error = None;
                    });
                    chime(&settings, audio::beep::ChimeKind::Start);
                    active = Some(Active {
                        settings,
                        entries,
                        target,
                        app,
                        started,
                    });
                }
                Action::Finish => {
                    controller.recording = None;
                    runtime::update(|s| {
                        s.recording = false;
                        s.recording_started = None;
                        s.mode = "processing".into();
                    });
                    if let Some(current) = active.take() {
                        match finish(mic, current, controller.generation, next_id, at) {
                            Ok(job) => {
                                next_id += 1;
                                match tx.try_send(job) {
                                    Ok(()) => controller.outstanding += 1,
                                    Err(mpsc::TrySendError::Full(job)) => {
                                        runtime::remember_audio(
                                            job.clip,
                                            job.settings,
                                            job.entries,
                                        );
                                        error("Inference queue is full; retry retained audio when available.");
                                    }
                                    Err(mpsc::TrySendError::Disconnected(_)) => {
                                        error("Inference worker stopped; restart Parla.")
                                    }
                                }
                            }
                            Err(e) => error(e),
                        }
                    }
                }
                Action::Cancel => {
                    restore_deadline = None;
                    generation.store(controller.generation, Ordering::Release);
                    controller.recording = None;
                    if active.take().is_some() {
                        let _ = mic.stop_at(at, Duration::ZERO);
                    }
                    controller.outstanding = controller.outstanding.saturating_sub(ready.len());
                    ready.clear();
                    ledger.clear();
                    session.forget();
                    runtime::purge();
                    runtime::update(|s| {
                        s.recording = false;
                        s.recording_started = None;
                        s.mode = "idle".into();
                    });
                }
                Action::None => {
                    if active.is_none()
                        && matches!(event, Some(TriggerEvent::PttStart | TriggerEvent::Toggle))
                        && controller.outstanding >= capacity
                    {
                        error("Transcription queue is full; wait for a result before recording again.");
                    }
                }
            }
        }
        while let Ok(done) = result_rx.try_recv() {
            if done.job.generation != controller.generation {
                controller.outstanding = controller.outstanding.saturating_sub(1);
                continue;
            }
            ready.push_back(done);
        }
        if active.is_none() && inject::transaction::modifiers_released() {
            if let Some(mut done) = ready.pop_front() {
                controller.outstanding = controller.outstanding.saturating_sub(1);
                runtime::remember_audio(
                    done.job.clip.clone(),
                    done.job.settings.clone(),
                    done.job.entries.clone(),
                );
                let t = Instant::now();
                if let Some(text) = done
                    .result
                    .final_text
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                {
                    if let Some(mut target) = done.job.target.take() {
                        for receipt in &ledger {
                            if receipt.verified && target.unchanged(&receipt.before) {
                                target = receipt.after.clone();
                            }
                        }
                        // Model cleanup must never turn dictated content into
                        // a destructive command. Require the original command.
                        if let Some(cmd) = command::classify_unchanged(
                            done.result.normalized.as_deref().unwrap_or(""),
                            &text,
                        ) {
                            let result = if !target.matches_current(120) {
                                Err("Command target changed.".into())
                            } else {
                                match cmd {
                                    command::Command::ScratchThat => {
                                        command::execute_scratch(&mut session)
                                    }
                                    command::Command::Bullets => {
                                        command::execute_bullets(&mut session)
                                    }
                                }
                            };
                            match result {
                                Ok(_) => done.result.insertion = "command".into(),
                                Err(e) => {
                                    done.result.insertion = "recovered".into();
                                    done.result.error = Some(e);
                                }
                            }
                        } else {
                            session.forget();
                            let payload = seam_spaced(&text);
                            match inject::transaction::commit(&target, &payload) {
                                Ok(receipt) => {
                                    done.result.insertion = if receipt.verified {
                                        "verified"
                                    } else {
                                        "accepted_unverified"
                                    }
                                    .into();
                                    last_original =
                                        seam_spaced(done.result.raw.as_deref().unwrap_or(&text));
                                    if receipt.verified {
                                        ledger.push_back(receipt.clone());
                                        while ledger.len() > capacity + 2 {
                                            ledger.pop_front();
                                        }
                                    }
                                    session.remember_receipt(&text, receipt);
                                    history.record_from_settings(&text);
                                }
                                Err(e) => {
                                    done.result.insertion = "recovered".into();
                                    done.result.error = Some(e);
                                    ledger.clear();
                                }
                            }
                        }
                    } else {
                        done.result.insertion = "preview".into();
                    }
                } else {
                    if done.result.insertion != "empty" {
                        done.result.insertion = "failed".into();
                    }
                    session.forget();
                }
                done.result.stage_ms["commit"] = serde_json::json!(t.elapsed().as_millis());
                done.result.stage_ms["release_to_finish"] = serde_json::json!(
                    done.job.queued_at.elapsed().as_millis() + done.job.finalize_ms
                );
                runtime::update(|s| {
                    if done.result.insertion == "recovered" {
                        s.error = Some("Text ready: open Parla and use Copy final".into());
                    }
                    s.last = done.result;
                });
            }
        }
        if let Some(deadline) = restore_deadline {
            if Instant::now() >= deadline {
                restore_deadline = None;
                error("Restore expired; use Copy raw or request it again.");
            } else if active.is_none()
                && controller.outstanding == 0
                && inject::transaction::modifiers_released()
                && session
                    .restore_target()
                    .is_some_and(|target| target.native_still_focused())
            {
                restore_deadline = None;
                match session.restore_original(&last_original) {
                    Ok(()) => runtime::update(|s| {
                        s.error = None;
                        s.last.final_text = s.last.raw.clone();
                        s.last.insertion = "restored".into();
                    }),
                    Err(e) => error(e),
                }
            }
        }
        runtime::update(|s| {
            s.queued = controller.outstanding;
            if !s.recording {
                s.mode = if controller.outstanding > 0 {
                    "processing"
                } else {
                    "idle"
                }
                .into();
            }
        });
        std::thread::sleep(Duration::from_millis(15));
    }
}
pub fn seam_spaced(text: &str) -> String {
    if text.is_empty() {
        String::new()
    } else if text.ends_with(char::is_whitespace) || text.bytes().all(|c| c.is_ascii_digit()) {
        text.into()
    } else {
        format!("{text} ")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toggle_and_hold_are_independent() {
        let mut c = Controller::default();
        assert_eq!(
            c.event(&TriggerEvent::Toggle, 3),
            Action::Start(RecordingMode::Toggle)
        );
        c.recording = Some(RecordingMode::Toggle);
        assert_eq!(c.event(&TriggerEvent::PttEnd, 3), Action::None);
        assert_eq!(c.event(&TriggerEvent::Toggle, 3), Action::Finish);
        assert_eq!(c.generation, 0);
        c.recording = Some(RecordingMode::Hold);
        assert_eq!(c.event(&TriggerEvent::Toggle, 3), Action::Finish);
        assert_eq!(c.event(&TriggerEvent::Cancel, 3), Action::Cancel);
        assert_eq!(c.generation, 1);
    }
    #[test]
    fn queue_full_refuses_new_recording_without_invalidating_jobs() {
        let mut c = Controller {
            outstanding: 3,
            ..Default::default()
        };
        assert_eq!(c.event(&TriggerEvent::Toggle, 3), Action::None);
        assert_eq!(c.generation, 0);
        assert_eq!(c.outstanding, 3);
        c.outstanding = 2;
        assert_eq!(
            c.event(&TriggerEvent::PttStart, 3),
            Action::Start(RecordingMode::Hold)
        );
    }
    #[test]
    fn spacing_is_exact_including_empty_and_existing_newline() {
        assert_eq!(seam_spaced(""), "");
        assert_eq!(seam_spaced("Hi"), "Hi ");
        assert_eq!(seam_spaced("Hi\n"), "Hi\n");
        assert_eq!(seam_spaced("154879132"), "154879132");
        assert_eq!(seam_spaced("007"), "007");
        assert_eq!(seam_spaced("one five"), "one five ");
        assert_eq!(seam_spaced("I need 5"), "I need 5 ");
    }
}
