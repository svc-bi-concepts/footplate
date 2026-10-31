//! The harness's audio: what the model hears and what it plays, as mono
//! PCM16 at 24 kHz (decision 2 of the design), whatever the devices speak.
//!
//! The rules are plain and tested without a device: the resampler, the level
//! gate that notices the person speaking, and the playback queue, which can be
//! dropped whole in one call and counts what was received, played and dropped.
//! The devices are cpal's (`device`, behind the `device` feature, built on the
//! device's own format and never asking it to change its rate) or fakes that
//! run on a clock (`start_fake`), which every test and the keyless rehearsal use.

use std::collections::VecDeque;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Where the microphone's chunks go: the session's channel, or a test's.
pub type MicSink = Box<dyn Fn(Vec<i16>) + Send>;

pub const RATE: u32 = 24_000;
/// 20 ms at 24 kHz: one append of audio.
pub const CHUNK: usize = 480;

#[cfg(feature = "device")]
pub fn to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

#[cfg(feature = "device")]
pub fn to_f32(x: i16) -> f32 {
    x as f32 / 32768.0
}

pub fn samples_ms(samples: u64) -> u64 {
    samples * 1000 / RATE as u64
}

/// Level of a chunk, in dB below full scale.
pub fn rms_dbfs(pcm: &[i16]) -> f32 {
    if pcm.is_empty() {
        return -120.0;
    }
    let sum: f64 = pcm.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum();
    let rms = (sum / pcm.len() as f64).sqrt();
    if rms <= 1e-6 {
        -120.0
    } else {
        (20.0 * rms.log10()) as f32
    }
}

/// A streaming resampler for one channel, from `from` Hz to `to` Hz. Down it
/// averages (a box filter: it takes the aliasing out of speech well enough,
/// which is all this is for), up it interpolates linearly. Spike-grade: the
/// product should have a proper polyphase filter or the rubato crate.
#[cfg(any(feature = "device", test))]
pub struct Resampler {
    ratio: f64,
    pos: f64,
    inp: Vec<f32>,
}

#[cfg(any(feature = "device", test))]
impl Resampler {
    pub fn new(from: u32, to: u32) -> Resampler {
        Resampler {
            ratio: from as f64 / to as f64,
            pos: 0.0,
            inp: Vec::new(),
        }
    }

    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.inp.extend_from_slice(input);
        if self.ratio > 1.0 {
            loop {
                let end = self.pos + self.ratio;
                let last = end.ceil() as usize;
                if last > self.inp.len() {
                    break;
                }
                let mut sum = 0.0f64;
                for i in self.pos.floor() as usize..last {
                    let lo = (i as f64).max(self.pos);
                    let hi = ((i + 1) as f64).min(end);
                    sum += (hi - lo).max(0.0) * self.inp[i] as f64;
                }
                out.push((sum / self.ratio) as f32);
                self.pos = end;
            }
        } else {
            loop {
                let i = self.pos.floor() as usize;
                if i + 1 >= self.inp.len() {
                    break;
                }
                let frac = (self.pos - i as f64) as f32;
                out.push(self.inp[i] * (1.0 - frac) + self.inp[i + 1] * frac);
                self.pos += self.ratio;
            }
        }
        let used = (self.pos.floor() as usize).min(self.inp.len());
        self.inp.drain(..used);
        self.pos -= used as f64;
    }
}

/// Notices the person speaking: a run of loud chunks is an onset, a longer
/// run of quiet ones the end. A level gate and nothing smarter, so a speaker
/// without a headset sets it off with the model's own voice (the harness says
/// so where it matters).
pub struct Vad {
    threshold_db: f32,
    run: u32,
    quiet: u32,
    speaking: bool,
}

#[derive(Debug, PartialEq)]
pub enum VadEvent {
    Onset,
    Offset,
}

/// Loud chunks in a row that make an onset: 60 ms.
pub const ONSET_CHUNKS: u32 = 3;
const OFFSET_CHUNKS: u32 = 15;

impl Vad {
    pub fn new(threshold_db: f32) -> Vad {
        Vad {
            threshold_db,
            run: 0,
            quiet: 0,
            speaking: false,
        }
    }

    pub fn feed(&mut self, chunk: &[i16]) -> Option<VadEvent> {
        if rms_dbfs(chunk) >= self.threshold_db {
            self.quiet = 0;
            self.run += 1;
            if !self.speaking && self.run >= ONSET_CHUNKS {
                self.speaking = true;
                return Some(VadEvent::Onset);
            }
        } else {
            self.run = 0;
            if self.speaking {
                self.quiet += 1;
                if self.quiet >= OFFSET_CHUNKS {
                    self.speaking = false;
                    return Some(VadEvent::Offset);
                }
            }
        }
        None
    }
}

/// What the model said, waiting to be played: pushed as it arrives, pulled by
/// the device's callback, droppable whole.
#[derive(Default)]
pub struct PlayQueue {
    inner: Mutex<Inner>,
    generation: AtomicU64,
}

#[derive(Default)]
struct Inner {
    q: VecDeque<i16>,
    rx: u64,
    played: u64,
    dropped: u64,
    last_audio: Option<Instant>,
    latency: Duration,
    /// Whether the last pull took audio, and when a pull first took none after
    /// one that did: the moments the sound stopped.
    was_playing: bool,
    silences: VecDeque<Instant>,
}

#[derive(Clone, Debug)]
pub struct PlayStats {
    pub rx_ms: u64,
    pub played_ms: u64,
    pub queued_ms: u64,
    pub dropped_ms: u64,
    /// The last time a callback took audio out of the queue.
    pub last_audio: Option<Instant>,
    /// What the device says lies between a callback's data and the speaker.
    pub latency: Duration,
}

impl PlayQueue {
    pub fn new() -> Arc<PlayQueue> {
        Arc::new(PlayQueue::default())
    }

    pub fn push(&self, pcm: &[i16]) {
        let mut inner = self.inner.lock().unwrap();
        inner.rx += pcm.len() as u64;
        inner.q.extend(pcm);
    }

    /// Fills `out` from the queue, with silence where it runs short; returns
    /// how many samples were audio.
    pub fn pull(&self, out: &mut [i16], now: Instant) -> usize {
        let mut inner = self.inner.lock().unwrap();
        let real = out.len().min(inner.q.len());
        for (slot, sample) in out.iter_mut().zip(inner.q.drain(..real)) {
            *slot = sample;
        }
        out[real..].fill(0);
        inner.played += real as u64;
        if real > 0 {
            inner.last_audio = Some(now);
        } else if inner.was_playing {
            inner.silences.push_back(now);
            if inner.silences.len() > 64 {
                inner.silences.pop_front();
            }
        }
        inner.was_playing = real > 0;
        real
    }

    /// The first time at or after `t` that a pull found the queue empty after
    /// audio: when the sound stopped, as far as the queue can tell (the device's
    /// own latency comes on top).
    pub fn first_silence_after(&self, t: Instant) -> Option<Instant> {
        self.inner
            .lock()
            .unwrap()
            .silences
            .iter()
            .copied()
            .find(|s| *s >= t)
    }

    /// Throws away what is queued and not yet taken: the call an interruption
    /// makes. Returns the milliseconds it threw away.
    pub fn drop_all(&self) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        let n = inner.q.len() as u64;
        inner.q.clear();
        inner.dropped += n;
        self.generation.fetch_add(1, Ordering::SeqCst);
        samples_ms(n)
    }

    /// Changes whenever the queue is dropped, so a device callback knows to
    /// throw away what it holds beyond the queue.
    #[cfg(any(feature = "device", test))]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    #[cfg(feature = "device")]
    pub fn set_latency(&self, latency: Duration) {
        self.inner.lock().unwrap().latency = latency;
    }

    pub fn stats(&self) -> PlayStats {
        let inner = self.inner.lock().unwrap();
        PlayStats {
            rx_ms: samples_ms(inner.rx),
            played_ms: samples_ms(inner.played),
            queued_ms: samples_ms(inner.q.len() as u64),
            dropped_ms: samples_ms(inner.dropped),
            last_audio: inner.last_audio,
            latency: inner.latency,
        }
    }
}

/// Stops the devices when dropped.
pub struct Devices {
    pub input: String,
    pub output: String,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
    /// The fake microphone speaks while this is set (fake devices only).
    pub speak: Arc<AtomicBool>,
}

impl Drop for Devices {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// Devices that run on a clock and open nothing: a microphone that is silent,
/// or speaks (a tone) while `speak` is set, and a speaker that takes the queue
/// at the pace of real playback. Chunks of 20 ms, as the real ones are cut.
pub fn start_fake(mic: MicSink, queue: Arc<PlayQueue>) -> Devices {
    let stop = Arc::new(AtomicBool::new(false));
    let speak = Arc::new(AtomicBool::new(false));
    let tick = Duration::from_millis(20);
    let mic_thread = {
        let (stop, speak) = (stop.clone(), speak.clone());
        thread::spawn(move || {
            let start = Instant::now();
            let mut n = 0u32;
            let mut phase = 0usize;
            while !stop.load(Ordering::SeqCst) {
                let loud = speak.load(Ordering::SeqCst);
                let chunk: Vec<i16> = (0..CHUNK)
                    .map(|i| {
                        if loud {
                            let t = (phase + i) as f32 / RATE as f32;
                            (6000.0 * (2.0 * PI * 440.0 * t).sin()) as i16
                        } else {
                            0
                        }
                    })
                    .collect();
                phase += CHUNK;
                mic(chunk);
                n += 1;
                if let Some(wait) = (start + tick * n).checked_duration_since(Instant::now()) {
                    thread::sleep(wait);
                }
            }
        })
    };
    let sink_thread = {
        let stop = stop.clone();
        thread::spawn(move || {
            let start = Instant::now();
            let mut n = 0u32;
            let mut out = [0i16; CHUNK];
            while !stop.load(Ordering::SeqCst) {
                queue.pull(&mut out, Instant::now());
                n += 1;
                if let Some(wait) = (start + tick * n).checked_duration_since(Instant::now()) {
                    thread::sleep(wait);
                }
            }
        })
    };
    Devices {
        input: "fake microphone".into(),
        output: "fake speaker".into(),
        stop,
        threads: vec![mic_thread, sink_thread],
        speak,
    }
}

#[cfg(feature = "device")]
pub mod device {
    //! cpal's devices, opened on their own default format: whatever the
    //! microphone gives (mostly f32 at 44.1 or 48 kHz, one or more channels)
    //! is made mono at 24 kHz here, and the queue's 24 kHz mono is made
    //! whatever the speaker takes. cpal's CoreAudio backend offers f32 alone
    //! and, asked for another rate, would change the device's own for every
    //! program on the machine: so the rate is never asked for.

    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{SampleFormat, SizedSample};
    use std::sync::mpsc;

    pub struct Capture {
        channels: usize,
        resampler: Resampler,
        acc: Vec<f32>,
        tx: MicSink,
    }

    impl Capture {
        pub fn new(channels: usize, rate: u32, tx: MicSink) -> Capture {
            Capture {
                channels,
                resampler: Resampler::new(rate, RATE),
                acc: Vec::new(),
                tx,
            }
        }

        pub fn feed<T: Copy>(&mut self, data: &[T], to_f32: fn(T) -> f32) {
            let mono: Vec<f32> = data
                .chunks(self.channels)
                .map(|frame| frame.iter().map(|&s| to_f32(s)).sum::<f32>() / self.channels as f32)
                .collect();
            let mut out = Vec::new();
            self.resampler.process(&mono, &mut out);
            self.acc.extend(out);
            while self.acc.len() >= CHUNK {
                let chunk: Vec<i16> = self.acc.drain(..CHUNK).map(to_i16).collect();
                (self.tx)(chunk);
            }
        }
    }

    pub struct Render {
        channels: usize,
        resampler: Resampler,
        pending: VecDeque<f32>,
        queue: Arc<PlayQueue>,
        generation: u64,
        rate: u32,
    }

    impl Render {
        pub fn new(channels: usize, rate: u32, queue: Arc<PlayQueue>) -> Render {
            Render {
                channels,
                resampler: Resampler::new(RATE, rate),
                pending: VecDeque::new(),
                generation: queue.generation(),
                queue,
                rate,
            }
        }

        pub fn fill<T: Copy>(&mut self, out: &mut [T], from_f32: fn(f32) -> T, latency: Duration) {
            self.queue.set_latency(latency);
            let generation = self.queue.generation();
            if generation != self.generation {
                // The queue was dropped: what is held here goes with it.
                self.generation = generation;
                self.pending.clear();
                self.resampler = Resampler::new(RATE, self.rate);
            }
            let frames = out.len() / self.channels;
            while self.pending.len() < frames {
                let mut pcm = [0i16; 120];
                self.queue.pull(&mut pcm, Instant::now());
                let src: Vec<f32> = pcm.iter().map(|&s| to_f32(s)).collect();
                let mut converted = Vec::new();
                self.resampler.process(&src, &mut converted);
                self.pending.extend(converted);
            }
            for frame in out.chunks_mut(self.channels) {
                frame.fill(from_f32(self.pending.pop_front().unwrap_or(0.0)));
            }
        }
    }

    fn describe(config: &cpal::SupportedStreamConfig) -> String {
        format!(
            "{} Hz, {} ch, {}",
            config.sample_rate(),
            config.channels(),
            config.sample_format()
        )
    }

    fn name(device: &cpal::Device) -> String {
        device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "an unnamed device".into())
    }

    fn input_stream<T: SizedSample + Send + 'static>(
        device: &cpal::Device,
        config: &cpal::SupportedStreamConfig,
        to_f32: fn(T) -> f32,
        tx: MicSink,
    ) -> Result<cpal::Stream, String> {
        let mut capture = Capture::new(config.channels() as usize, config.sample_rate(), tx);
        device
            .build_input_stream(
                (*config).into(),
                move |data: &[T], _: &cpal::InputCallbackInfo| capture.feed(data, to_f32),
                |err| eprintln!("microphone stream: {err}"),
                None,
            )
            .map_err(|err| format!("the microphone could not be opened: {err}"))
    }

    fn output_stream<T: SizedSample + Send + 'static>(
        device: &cpal::Device,
        config: &cpal::SupportedStreamConfig,
        from_f32: fn(f32) -> T,
        queue: Arc<PlayQueue>,
    ) -> Result<cpal::Stream, String> {
        let mut render = Render::new(config.channels() as usize, config.sample_rate(), queue);
        device
            .build_output_stream(
                (*config).into(),
                move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                    let ts = info.timestamp();
                    render.fill(data, from_f32, ts.playback.duration_since(ts.callback));
                },
                |err| eprintln!("speaker stream: {err}"),
                None,
            )
            .map_err(|err| format!("the speaker could not be opened: {err}"))
    }

    /// Opens the default microphone and speaker. The streams live on a thread
    /// of their own (cpal's are not `Send`) until the returned value drops.
    pub fn start(mic: MicSink, queue: Arc<PlayQueue>) -> Result<Devices, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = {
            let stop = stop.clone();
            thread::spawn(move || {
                let built = (|| -> Result<(String, String, cpal::Stream, cpal::Stream), String> {
                    let host = cpal::default_host();
                    let input = host
                        .default_input_device()
                        .ok_or("this machine has no default microphone")?;
                    let output = host
                        .default_output_device()
                        .ok_or("this machine has no default speaker")?;
                    let in_config = input
                        .default_input_config()
                        .map_err(|err| format!("the microphone's format: {err}"))?;
                    let out_config = output
                        .default_output_config()
                        .map_err(|err| format!("the speaker's format: {err}"))?;
                    let in_stream = match in_config.sample_format() {
                        SampleFormat::F32 => input_stream::<f32>(&input, &in_config, |s| s, mic)?,
                        SampleFormat::I16 => input_stream::<i16>(&input, &in_config, to_f32, mic)?,
                        other => return Err(format!("the microphone's {other} is not handled")),
                    };
                    let out_stream = match out_config.sample_format() {
                        SampleFormat::F32 => {
                            output_stream::<f32>(&output, &out_config, |s| s, queue)?
                        }
                        SampleFormat::I16 => {
                            output_stream::<i16>(&output, &out_config, to_i16, queue)?
                        }
                        other => return Err(format!("the speaker's {other} is not handled")),
                    };
                    in_stream
                        .play()
                        .map_err(|err| format!("the microphone would not start: {err}"))?;
                    out_stream
                        .play()
                        .map_err(|err| format!("the speaker would not start: {err}"))?;
                    Ok((
                        format!("{} ({})", name(&input), describe(&in_config)),
                        format!("{} ({})", name(&output), describe(&out_config)),
                        in_stream,
                        out_stream,
                    ))
                })();
                match built {
                    Ok((input, output, in_stream, out_stream)) => {
                        let _ = ready_tx.send(Ok((input, output)));
                        while !stop.load(Ordering::SeqCst) {
                            thread::sleep(Duration::from_millis(50));
                        }
                        drop(in_stream);
                        drop(out_stream);
                    }
                    Err(why) => {
                        let _ = ready_tx.send(Err(why));
                    }
                }
            })
        };
        match ready_rx.recv() {
            Ok(Ok((input, output))) => Ok(Devices {
                input,
                output,
                stop,
                threads: vec![thread],
                speak: Arc::new(AtomicBool::new(false)),
            }),
            Ok(Err(why)) => {
                let _ = thread.join();
                Err(why)
            }
            Err(_) => Err("the audio thread ended before it opened a device".into()),
        }
    }

    /// What the default devices are and take, without opening a stream (a
    /// microphone's consent is asked for when one is opened, not here); with
    /// `play_silence`, also a second of silence through the speaker, to read
    /// the callback's size and the device's own latency from.
    pub fn probe(play_silence: bool) -> String {
        let mut text = String::new();
        let host = cpal::default_host();
        text += &format!("host: {}\n", host.id());
        for (label, device, input) in [
            ("input", host.default_input_device(), true),
            ("output", host.default_output_device(), false),
        ] {
            let Some(device) = device else {
                text += &format!("{label}: none\n");
                continue;
            };
            text += &format!("{label}: {}\n", name(&device));
            let default = if input {
                device.default_input_config()
            } else {
                device.default_output_config()
            };
            match default {
                Ok(config) => text += &format!("  default: {}\n", describe(&config)),
                Err(err) => text += &format!("  default: {err}\n"),
            }
            let offers: Vec<(u16, u32, u32, SampleFormat)> = if input {
                device
                    .supported_input_configs()
                    .map(|all| {
                        all.map(|c| {
                            (
                                c.channels(),
                                c.min_sample_rate(),
                                c.max_sample_rate(),
                                c.sample_format(),
                            )
                        })
                        .collect()
                    })
                    .unwrap_or_default()
            } else {
                device
                    .supported_output_configs()
                    .map(|all| {
                        all.map(|c| {
                            (
                                c.channels(),
                                c.min_sample_rate(),
                                c.max_sample_rate(),
                                c.sample_format(),
                            )
                        })
                        .collect()
                    })
                    .unwrap_or_default()
            };
            let lines: Vec<String> = offers
                .iter()
                .map(|(ch, lo, hi, format)| {
                    if lo == hi {
                        format!("{format} x{ch} at {lo} Hz")
                    } else {
                        format!("{format} x{ch} at {lo}-{hi} Hz")
                    }
                })
                .collect();
            text += &format!("  offers: {}\n", lines.join("; "));
            let native = |format: SampleFormat, channels: u16| {
                offers.iter().any(|(ch, lo, hi, f)| {
                    *f == format && *ch == channels && (*lo..=*hi).contains(&RATE)
                })
            };
            text += &format!(
                "  mono i16 at 24000 Hz as cpal offers it: {}; mono f32 at 24000 Hz: {}\n",
                if native(SampleFormat::I16, 1) {
                    "yes"
                } else {
                    "no"
                },
                if native(SampleFormat::F32, 1) {
                    "yes"
                } else {
                    "no"
                },
            );
        }
        if play_silence {
            text += &probe_silence();
            text += &probe_playback();
        }
        text
    }

    /// The harness's own playback path on the real speaker, silently: two
    /// seconds of zeros are queued as if they were speech, played for half a
    /// second, and dropped. It reads off how well the queue's count keeps up
    /// with the clock and how soon after the drop the callback found nothing
    /// to play, which with the device's own latency is when a speaker falls
    /// silent after an interruption.
    fn probe_playback() -> String {
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            return "playback: no speaker\n".into();
        };
        let Ok(config) = device.default_output_config() else {
            return "playback: no format\n".into();
        };
        if config.sample_format() != SampleFormat::F32 {
            return format!(
                "playback: skipped, the speaker takes {}\n",
                config.sample_format()
            );
        }
        let queue = PlayQueue::new();
        let stream = match output_stream::<f32>(&device, &config, |s| s, queue.clone()) {
            Ok(stream) => stream,
            Err(why) => return format!("playback: {why}\n"),
        };
        if let Err(err) = stream.play() {
            return format!("playback: would not start: {err}\n");
        }
        thread::sleep(Duration::from_millis(100));
        queue.push(&vec![0i16; 2 * RATE as usize]);
        let pushed_at = Instant::now();
        thread::sleep(Duration::from_millis(500));
        let played = queue.stats().played_ms;
        let waited = pushed_at.elapsed();
        let dropped_at = Instant::now();
        let dropped = queue.drop_all();
        thread::sleep(Duration::from_millis(300));
        let silent = queue.first_silence_after(dropped_at);
        let latency = queue.stats().latency;
        drop(stream);
        match silent {
            Some(at) => {
                let to_callback = at.duration_since(dropped_at);
                format!(
                    "playback: queued 2000 ms, {} ms of the clock later {played} ms counted as played; dropped {dropped} ms; \
                     the callback found nothing {} ms after the drop, and the device's own latency is {} ms: \
                     the speaker is silent about {} ms after a drop\n",
                    waited.as_millis(),
                    to_callback.as_millis(),
                    latency.as_millis(),
                    (to_callback + latency).as_millis()
                )
            }
            None => "playback: the callback never found the queue empty after the drop\n".into(),
        }
    }

    fn probe_silence() -> String {
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            return "silence: no speaker\n".into();
        };
        let Ok(config) = device.default_output_config() else {
            return "silence: no format\n".into();
        };
        if config.sample_format() != SampleFormat::F32 {
            return format!(
                "silence: skipped, the speaker takes {}\n",
                config.sample_format()
            );
        }
        let seen = Arc::new(Mutex::new((0usize, Vec::<usize>::new(), Duration::ZERO)));
        let sink = seen.clone();
        let channels = config.channels() as usize;
        let stream = device.build_output_stream(
            config.into(),
            move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                data.fill(0.0);
                let ts = info.timestamp();
                let mut seen = sink.lock().unwrap();
                seen.0 += 1;
                seen.1.push(data.len() / channels);
                seen.2 = ts.playback.duration_since(ts.callback);
            },
            |err| eprintln!("speaker stream: {err}"),
            None,
        );
        match stream {
            Err(err) => format!("silence: the speaker could not be opened: {err}\n"),
            Ok(stream) => {
                if let Err(err) = stream.play() {
                    return format!("silence: would not start: {err}\n");
                }
                let began = Instant::now();
                thread::sleep(Duration::from_secs(1));
                drop(stream);
                let elapsed = began.elapsed();
                let (calls, sizes, latency) = seen.lock().unwrap().clone();
                let total: usize = sizes.iter().sum();
                let rate = config.sample_rate() as f64;
                format!(
                    "silence: {calls} callbacks in {} ms of the clock; {} to {} frames each; \
                     {total} frames delivered, which is {:.0} ms at {} Hz, {:+.0} ms against the clock; \
                     device latency {} ms\n",
                    elapsed.as_millis(),
                    sizes.iter().min().copied().unwrap_or(0),
                    sizes.iter().max().copied().unwrap_or(0),
                    total as f64 / rate * 1000.0,
                    config.sample_rate(),
                    total as f64 / rate * 1000.0 - elapsed.as_secs_f64() * 1000.0,
                    latency.as_millis()
                )
            }
        }
    }
}

#[cfg(all(test, feature = "device"))]
mod device_tests {
    use super::device::{Capture, Render};
    use super::*;
    use std::sync::mpsc;

    fn stereo_sine(hz: f32, rate: u32, seconds: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize)
            .flat_map(|i| {
                let v = 0.5 * (2.0 * PI * hz * i as f32 / rate as f32).sin();
                [v, v]
            })
            .collect()
    }

    #[test]
    fn a_microphone_at_48k_in_stereo_becomes_chunks_of_mono_24k() {
        let (tx, rx) = mpsc::channel();
        let mut capture = Capture::new(2, 48_000, Box::new(move |chunk| tx.send(chunk).unwrap()));
        // A second of tone, as a device calls back: pieces of 512 frames.
        for piece in stereo_sine(1000.0, 48_000, 1.0).chunks(1024) {
            capture.feed(piece, |s| s);
        }
        let chunks: Vec<Vec<i16>> = rx.try_iter().collect();
        assert!(chunks.len().abs_diff(50) <= 1, "{} chunks", chunks.len());
        assert!(chunks.iter().all(|c| c.len() == CHUNK));
        let level = rms_dbfs(&chunks[10]);
        assert!((level + 9.0).abs() < 1.0, "{level} dBFS for a 0.5 sine");
        // An i16 device is the same through another conversion.
        let (tx, rx) = mpsc::channel();
        let mut capture = Capture::new(1, 24_000, Box::new(move |chunk| tx.send(chunk).unwrap()));
        capture.feed(&vec![16384i16; 4800], to_f32);
        assert!(rx.try_iter().count() >= 9);
    }

    #[test]
    fn the_queue_comes_out_stereo_at_the_speakers_rate_and_a_drop_is_silence() {
        let queue = PlayQueue::new();
        let mut render = Render::new(2, 48_000, queue.clone());
        let tone: Vec<i16> = (0..24_000)
            .map(|i| (12000.0 * (2.0 * PI * 1000.0 * i as f32 / 24_000.0).sin()) as i16)
            .collect();
        queue.push(&tone);
        let mut heard = Vec::new();
        for _ in 0..20 {
            let mut buffer = vec![0f32; 1024];
            render.fill(&mut buffer, |s| s, Duration::from_millis(12));
            heard.extend(buffer);
        }
        // 20 buffers of 512 frames at 48 kHz are 213 ms: 5120 samples of the
        // queue at 24 kHz, both channels alike, the tone's level kept.
        assert!(heard.chunks(2).all(|frame| frame[0] == frame[1]));
        let left: Vec<f32> = heard.chunks(2).map(|f| f[0]).collect();
        let level =
            (left[200..].iter().map(|v| v * v).sum::<f32>() / (left.len() - 200) as f32).sqrt();
        assert!((level - 0.2543).abs() < 0.03, "level {level}");
        let stats = queue.stats();
        assert!(
            (stats.played_ms as i64 - 213).abs() < 12,
            "{} ms counted as played",
            stats.played_ms
        );
        assert_eq!(stats.latency, Duration::from_millis(12));
        // Dropped, the next callback is silence at once, though it held audio.
        queue.drop_all();
        let mut buffer = vec![1f32; 1024];
        render.fill(&mut buffer, |s| s, Duration::ZERO);
        assert!(buffer.iter().all(|v| *v == 0.0), "silence after a drop");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(hz: f32, rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| 0.5 * (2.0 * PI * hz * i as f32 / rate as f32).sin())
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn a_resampler_keeps_a_tone_and_the_count() {
        for from in [48_000u32, 44_100, 16_000, 24_000] {
            let mut resampler = Resampler::new(from, RATE);
            let mut out = Vec::new();
            // Fed in uneven pieces, as a device calls back.
            let tone = sine(1000.0, from, from as usize);
            for piece in tone.chunks(333) {
                resampler.process(piece, &mut out);
            }
            let expected = RATE as usize;
            assert!(
                out.len().abs_diff(expected) <= 3,
                "{from}: {} samples",
                out.len()
            );
            let level = rms(&out[100..out.len() - 100]);
            assert!((level - 0.3535).abs() < 0.04, "{from}: level {level}");
        }
    }

    #[test]
    fn a_resampler_goes_up_by_interpolating() {
        let mut resampler = Resampler::new(24_000, 48_000);
        let mut out = Vec::new();
        resampler.process(&[0.0, 1.0, 2.0, 3.0], &mut out);
        assert_eq!(out, [0.0, 0.5, 1.0, 1.5, 2.0, 2.5]);
        // And a constant stays one when brought down.
        let mut resampler = Resampler::new(44_100, RATE);
        let mut out = Vec::new();
        resampler.process(&vec![0.25; 4410], &mut out);
        assert!(out.iter().all(|v| (v - 0.25).abs() < 1e-5));
    }

    #[test]
    fn levels_are_in_dbfs() {
        assert_eq!(rms_dbfs(&[]), -120.0);
        assert_eq!(rms_dbfs(&[0; 480]), -120.0);
        let loud = vec![16384i16; 480];
        assert!((rms_dbfs(&loud) + 6.02).abs() < 0.05);
    }

    #[test]
    fn the_gate_notices_speech_and_its_end() {
        let mut vad = Vad::new(-35.0);
        let quiet = [0i16; CHUNK];
        let loud = [8000i16; CHUNK];
        for _ in 0..10 {
            assert_eq!(vad.feed(&quiet), None);
        }
        assert_eq!(vad.feed(&loud), None);
        assert_eq!(vad.feed(&loud), None);
        assert_eq!(vad.feed(&loud), Some(VadEvent::Onset));
        // Speech goes on without a second onset, through a short pause.
        assert_eq!(vad.feed(&loud), None);
        for _ in 0..OFFSET_CHUNKS - 1 {
            assert_eq!(vad.feed(&quiet), None);
        }
        assert_eq!(vad.feed(&loud), None);
        for _ in 0..OFFSET_CHUNKS - 1 {
            assert_eq!(vad.feed(&quiet), None);
        }
        assert_eq!(vad.feed(&quiet), Some(VadEvent::Offset));
        // A single click is no onset.
        assert_eq!(vad.feed(&loud), None);
        assert_eq!(vad.feed(&quiet), None);
    }

    #[test]
    fn a_queue_plays_in_order_counts_and_drops_whole() {
        let queue = PlayQueue::new();
        queue.push(&(1..=1000).collect::<Vec<i16>>());
        let mut out = [0i16; 400];
        assert_eq!(queue.pull(&mut out, Instant::now()), 400);
        assert_eq!(out[0], 1);
        assert_eq!(out[399], 400);
        let stats = queue.stats();
        assert_eq!((stats.rx_ms, stats.played_ms), (41, 16));
        assert_eq!(stats.queued_ms, 25);
        let generation = queue.generation();
        // The drop takes what is left and tells the callback to let go of its own.
        assert_eq!(queue.drop_all(), 25);
        assert_ne!(queue.generation(), generation);
        assert_eq!(queue.stats().dropped_ms, 25);
        assert_eq!(queue.pull(&mut out, Instant::now()), 0);
        assert!(
            out.iter().all(|&s| s == 0),
            "silence where nothing is queued"
        );
        // Silence does not count as played.
        assert_eq!(queue.stats().played_ms, 16);
        // The first pull to find nothing after audio is when the sound stopped.
        let mut out = [0i16; 10];
        let (before, after) = (Instant::now(), Instant::now() + Duration::from_secs(60));
        queue.push(&[5; 20]);
        queue.pull(&mut out, Instant::now());
        assert_eq!(queue.first_silence_after(before), None, "still playing");
        queue.pull(&mut out, Instant::now());
        queue.pull(&mut out, Instant::now());
        assert_eq!(queue.pull(&mut out, Instant::now()), 0);
        let silent = queue.first_silence_after(before).expect("went quiet");
        assert!(silent >= before);
        assert_eq!(queue.first_silence_after(after), None);
    }
}
