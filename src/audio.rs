//! The microphone and the speaker of a voice session: what the session
//! hears through ([`crate::voice::Mic`]) and speaks through ([`Queue`]), as
//! mono PCM16 at [`RATE`] whatever the devices speak. It moves PCM frames and
//! nothing else: no provider, event or key is known here, so a second voice,
//! if one comes, finds the same two ends.
//!
//! The rules are plain and tested on any system with no device: the
//! [`Resampler`], [`Capture`] (a microphone's callbacks made mono, brought to
//! the socket's rate and cut into the 20 ms frames the session sends),
//! [`Render`] (the queue brought to the speaker's rate and channels, letting
//! go of what it holds when the queue is dropped), the names a person
//! tells the devices by ([`named`]) and which microphone a kept choice opens
//! ([`pick`]). They are the harness's
//! (spikes/voice-harness/src/audio.rs), which runs 3 to 6 ran live on a
//! Linux desktop; the tests stand in for the devices by calling them as a
//! device's callbacks do.
//!
//! The devices are cpal's, behind `cfg`: CoreAudio on macOS through bindings
//! (no C library) and ALSA on Linux, which is the one C library this adds
//! (`libasound`). They are opened on their own default format and never asked
//! for another rate: cpal's CoreAudio would change the device's own rate for
//! every program on the machine, so the conversion is here. Elsewhere
//! [`open`] says there is no audio yet, as Windows has none.
//!
//! A Mac's consent to the microphone is not the devices' to report: an app
//! without it is given buffers of zeros and no error, so a session would stand
//! and hear nobody. [`open`] reads the consent ([`Consent`]) before the
//! microphone is opened and again after, waiting for the system's first
//! answer, and refuses in words ([`allowed`]).

use std::collections::VecDeque;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use crate::voice::{Queue, RATE};

/// 20 ms at [`RATE`]: one frame of the microphone, as the session sends it.
pub const CHUNK: usize = 480;

/// How much of the queue the speaker takes at once: 5 ms, so that a drop is
/// heard as silence within the time one callback holds.
const PIECE: usize = 120;

pub(crate) fn to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

pub(crate) fn to_f32(x: i16) -> f32 {
    f32::from(x) / 32768.0
}

/// A streaming resampler for one channel, from `from` Hz to `to` Hz. Down it
/// averages (a box filter: it takes the aliasing out of speech well enough,
/// which is all this is for), up it interpolates linearly. Runs 3 to 6 were
/// heard through this; a polyphase filter would be cleaner, and nothing that
/// was measured asks for one.
pub struct Resampler {
    ratio: f64,
    pos: f64,
    inp: Vec<f32>,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Resampler {
        Resampler {
            ratio: f64::from(from) / f64::from(to),
            pos: 0.0,
            inp: Vec::new(),
        }
    }

    /// Takes `input` and appends what it makes of it to `out`; what does not
    /// fill a sample of the output yet is kept for the next call.
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
                    sum += (hi - lo).max(0.0) * f64::from(self.inp[i]);
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

/// What a microphone gives, made what the session sends. The device's
/// callbacks arrive in its own format — interleaved channels at its rate, in
/// whatever size it chooses — and leave as frames of [`CHUNK`] mono samples at
/// [`RATE`], handed to `sink` as they fill.
pub struct Capture<F> {
    channels: usize,
    resampler: Resampler,
    mono: Vec<f32>,
    resampled: Vec<f32>,
    /// What is resampled and does not fill a frame yet.
    acc: Vec<f32>,
    sink: F,
}

impl<F: FnMut(Vec<i16>)> Capture<F> {
    pub fn new(channels: usize, rate: u32, sink: F) -> Capture<F> {
        Capture {
            channels: channels.max(1),
            resampler: Resampler::new(rate, RATE),
            mono: Vec::new(),
            resampled: Vec::new(),
            acc: Vec::new(),
            sink,
        }
    }

    /// One callback's samples, `T` in whatever the device's format is, and
    /// `to_f32` the way to -1.0 to 1.0. The channels are averaged: a
    /// microphone on one channel of several reads quieter, and is not lost.
    pub fn feed<T: Copy>(&mut self, data: &[T], to_f32: fn(T) -> f32) {
        let channels = self.channels as f32;
        self.mono.clear();
        self.mono.extend(
            data.chunks(self.channels)
                .map(|frame| frame.iter().map(|&s| to_f32(s)).sum::<f32>() / channels),
        );
        self.resampled.clear();
        self.resampler.process(&self.mono, &mut self.resampled);
        self.acc.extend_from_slice(&self.resampled);
        while self.acc.len() >= CHUNK {
            let frame: Vec<i16> = self.acc.drain(..CHUNK).map(to_i16).collect();
            (self.sink)(frame);
        }
    }
}

/// The queue's lock, whatever became of the thread that held it last: a panic
/// there is that session's, and a speaker's callback is no place to panic
/// (CoreAudio's is C's, and unwinding out of it aborts the program).
fn locked(queue: &Mutex<Queue>) -> MutexGuard<'_, Queue> {
    queue.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the voice says, made what a speaker takes: the session's [`Queue`]
/// at [`RATE`], mono, brought to the device's rate and spread over its
/// channels, a callback at a time. Where the queue has nothing the speaker
/// gets silence, and where the queue was dropped it lets go of what it held
/// beyond the queue — what it had taken and not yet played, and the
/// resampler's tail — so that the drop is heard at once.
pub struct Render {
    channels: usize,
    rate: u32,
    resampler: Resampler,
    /// Samples at the device's rate, taken from the queue and not yet given
    /// to the speaker.
    pending: VecDeque<f32>,
    converted: Vec<f32>,
    queue: Arc<Mutex<Queue>>,
    generation: u64,
}

impl Render {
    pub fn new(channels: usize, rate: u32, queue: Arc<Mutex<Queue>>) -> Render {
        let generation = locked(&queue).generation();
        Render {
            channels: channels.max(1),
            rate,
            resampler: Resampler::new(RATE, rate),
            pending: VecDeque::new(),
            converted: Vec::new(),
            queue,
            generation,
        }
    }

    fn discard_dropped(&mut self, generation: u64) {
        if generation != self.generation {
            self.generation = generation;
            self.pending.clear();
            self.resampler = Resampler::new(RATE, self.rate);
        }
    }

    /// Fills one callback's buffer `out`, `T` in whatever the device's format
    /// is and `from_f32` the way from -1.0 to 1.0.
    pub fn fill<T: Copy>(&mut self, out: &mut [T], from_f32: fn(f32) -> T) {
        // Even a short callback must notice a drop: it may fit entirely in
        // samples the resampler had already taken from the queue.
        let generation = locked(&self.queue).generation();
        self.discard_dropped(generation);
        let frames = out.len() / self.channels;
        while self.pending.len() < frames {
            let mut pcm = [0i16; PIECE];
            {
                let mut queue = locked(&self.queue);
                let generation = queue.generation();
                if generation != self.generation {
                    self.generation = generation;
                    self.pending.clear();
                    self.resampler = Resampler::new(RATE, self.rate);
                }
                queue.pull(&mut pcm);
            }
            let source = pcm.map(to_f32);
            self.converted.clear();
            self.resampler.process(&source, &mut self.converted);
            self.pending.extend(self.converted.iter().copied());
        }
        for frame in out.chunks_mut(self.channels) {
            frame.fill(from_f32(self.pending.pop_front().unwrap_or(0.0)));
        }
    }
}

/// How a device is named to the person, so that they can tell which
/// microphone and which speaker stand in the session: what cpal calls it,
/// the host's id for it, and the format it runs in. The default's own name
/// can be a generic one (ALSA's "Default Audio Device"), and on CoreAudio the
/// id is the device's UID, which names the real device behind the generic
/// default; with no id there is the description alone. The harness's DEVICES
/// line, which carried runs 3 to 6.
pub fn named(
    description: &str,
    id: Option<&str>,
    rate: u32,
    channels: u16,
    format: &str,
) -> String {
    let device = match id {
        Some(id) => format!("{description}, id {id}"),
        None => description.to_string(),
    };
    format!("{device} ({rate} Hz, {channels} ch, {format})")
}

/// A microphone the host lists, as the settings offer it: named as
/// [`named`] names a device, by what cpal calls it and the host's id for it,
/// so that a row of the list is the start of what the session reports once
/// it has opened it. The format is left to that report: a device tells it
/// only when it is opened, ALSA opens a device to read it, and listing must
/// open none (cpal's own listing is written not to, since a plugin that
/// refuses the open can leak descriptors for the life of the process).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    /// The host's id for it (`coreaudio:<UID>`, `alsa:<pcm id>`), which the
    /// settings keep: the UID names the same device after a restart, where
    /// its place in the list does not.
    pub id: String,
    /// What cpal calls it.
    pub description: String,
}

/// Which of the microphones the host lists a session opens for the id the
/// settings keep: the one with that id, or none, which is the system's
/// default, where nothing is kept or the kept id is not listed (the
/// microphone is unplugged, or the host names it otherwise now). The session
/// then names the microphone that stands ([`Devices::input`]), as it does for
/// the default.
pub fn pick(listed: &[Input], kept: &str) -> Option<usize> {
    if kept.is_empty() {
        return None;
    }
    listed.iter().position(|input| input.id == kept)
}

/// What the person is told where the system said no to the microphone: that it
/// did, and where that is changed, which cpal's own word for it
/// ("Unauthorized") names neither. A Mac keeps the answer to its consent
/// prompt in System Settings; on Linux the sound system's own setting decides,
/// and none is named.
pub fn refused(mac: bool) -> String {
    match mac {
        true => "the microphone may not be used: allow Footplate in System Settings, Privacy & Security, Microphone, and try again".to_string(),
        false => "the microphone may not be used: the system refused it to this account".to_string(),
    }
}

/// What a Mac says of this app's use of the microphone: AVFoundation's
/// authorization status for audio, in the four words it has. It is the
/// consent itself and not the devices' word for it, since an app without
/// consent is given zeros and no error: the devices open, and nobody is heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consent {
    /// Not asked yet: opening asks the system and waits for the person's answer.
    NotDetermined,
    /// Held back by a policy of the machine (parental controls, a profile),
    /// whatever the person would answer.
    Restricted,
    /// The person said no, at the prompt or in System Settings after it.
    Denied,
    Authorized,
}

/// Whether a session may open the microphone under `consent`, or the words
/// for the person where it may not: [`refused`]'s, for a Mac, whether the
/// person or a policy said no. `NotDetermined` may proceed after the system's
/// prompt has been answered; this function only maps the status to words.
pub fn allowed(consent: Consent) -> Result<(), String> {
    match consent {
        Consent::Denied | Consent::Restricted => Err(refused(true)),
        Consent::Authorized | Consent::NotDetermined => Ok(()),
    }
}

/// Resolves an unanswered consent through the system prompt. Unknown status
/// stays open as before, without asking; a known refusal never asks again.
fn resolve_consent(consent: Option<Consent>, request: impl FnOnce() -> bool) -> Consent {
    match consent {
        Some(Consent::NotDetermined) => match request() {
            true => Consent::Authorized,
            false => Consent::Denied,
        },
        Some(consent) => consent,
        None => Consent::NotDetermined,
    }
}

/// Runs `open` between two reads of the consent: before it, so that a
/// microphone already refused is not opened (and an unanswered consent gets
/// the system prompt and its answer first), and after it, in case consent
/// changed while the devices opened. The devices `open` made are dropped on
/// that second no, closed as they are when a later step of opening them fails.
/// An `open` that fails says its own words, and the consent is not read again.
fn consenting<T>(
    read: impl Fn() -> Consent,
    open: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    allowed(read())?;
    let opened = open()?;
    allowed(read())?;
    Ok(opened)
}

/// The devices of a session, open while this stands: the microphone feeding
/// the session and the speaker playing its [`Queue`]. Dropping it closes
/// both. The streams live on a thread of their own, since cpal's are
/// not `Send`.
pub struct Devices {
    /// The microphone and the speaker that stand in the session, as
    /// [`named`] says them.
    pub input: String,
    pub output: String,
    /// Closed to stop the thread, which waits on it holding the streams.
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Devices {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Opens the microphone and the default speaker for a session: the
/// microphone's frames, mono PCM16 at [`RATE`], go to `sink` (the session's
/// [`crate::voice::Mic`]), and the speaker plays `speaker` (the session's
/// [`crate::voice::Voice::speaker`]). The microphone is the one `microphone`
/// names, an [`Input::id`], and the system's default where it is empty or the
/// host no longer lists it ([`pick`]); the speaker is always the default's.
/// Where it cannot, the reason is said in words for the person (no
/// microphone, a format it does not handle, a stream that would not start,
/// and on a Mac the consent refused: [`allowed`], for the chosen microphone as
/// for the default). Windows has no voice yet.
///
/// It returns when the devices are open, and the first time a Mac waits here
/// for the person's answer to its microphone consent: it is no call for the
/// window's thread.
pub fn open(
    sink: impl FnMut(Vec<i16>) + Send + 'static,
    speaker: Arc<Mutex<Queue>>,
    microphone: &str,
) -> Result<Devices, String> {
    #[cfg(target_os = "macos")]
    {
        consenting(mac::consent, || device::open(sink, speaker, microphone))
    }
    #[cfg(target_os = "linux")]
    {
        device::open(sink, speaker, microphone)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (sink, speaker, microphone);
        Err("voice has no microphone or speaker on this system yet".to_string())
    }
}

/// The microphones this machine lists, in the host's order, for the settings
/// to offer: none where voice has no devices, as [`open`] refuses there. It
/// opens none of them, so a Mac asks for no consent, and it asks the host
/// what a host may be slow to answer: it is no call for the window's thread.
pub fn inputs() -> Vec<Input> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        device::inputs()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Vec::new()
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod device {
    //! cpal's devices, opened on their own default format: whatever the
    //! microphone gives (f32 or i16 at 44.1 or 48 kHz, one or more channels)
    //! is made mono at 24 kHz by [`Capture`], and the queue's 24 kHz mono is
    //! made whatever the speaker takes by [`Render`].

    use std::thread;

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{SampleFormat, SizedSample};

    use super::*;
    use crate::voice::say;

    fn description_of(device: &cpal::Device) -> String {
        device
            .description()
            .map(|d| d.to_string())
            .unwrap_or_else(|_| "an unnamed device".into())
    }

    fn describe(device: &cpal::Device, config: &cpal::SupportedStreamConfig) -> String {
        let id = device.id().ok().map(|id| id.to_string());
        named(
            &description_of(device),
            id.as_deref(),
            config.sample_rate(),
            config.channels(),
            &config.sample_format().to_string(),
        )
    }

    /// A stream's trouble is logged the first time it comes: a device that
    /// has gone says so on every callback.
    fn on_error(what: &'static str) -> impl FnMut(cpal::Error) + Send + 'static {
        let mut said = false;
        move |err| {
            if !std::mem::replace(&mut said, true) {
                say(&format!("voice {what}: {err}"));
            }
        }
    }

    /// The microphone's trouble as words for the person: the system's no is
    /// said as one, with where it is changed ([`refused`]), and anything
    /// else is cpal's own.
    fn trouble(what: &str, err: &cpal::Error) -> String {
        match err.kind() {
            cpal::ErrorKind::PermissionDenied => refused(cfg!(target_os = "macos")),
            _ => format!("{what}: {err}"),
        }
    }

    fn input_stream<T: SizedSample + Send + 'static>(
        device: &cpal::Device,
        config: &cpal::SupportedStreamConfig,
        to_f32: fn(T) -> f32,
        sink: impl FnMut(Vec<i16>) + Send + 'static,
    ) -> Result<cpal::Stream, String> {
        let mut capture = Capture::new(config.channels() as usize, config.sample_rate(), sink);
        device
            .build_input_stream(
                (*config).into(),
                move |data: &[T], _: &cpal::InputCallbackInfo| capture.feed(data, to_f32),
                on_error("microphone"),
                None,
            )
            .map_err(|err| trouble("the microphone could not be opened", &err))
    }

    fn output_stream<T: SizedSample + Send + 'static>(
        device: &cpal::Device,
        config: &cpal::SupportedStreamConfig,
        from_f32: fn(f32) -> T,
        queue: Arc<Mutex<Queue>>,
    ) -> Result<cpal::Stream, String> {
        let mut render = Render::new(config.channels() as usize, config.sample_rate(), queue);
        device
            .build_output_stream(
                (*config).into(),
                move |data: &mut [T], _: &cpal::OutputCallbackInfo| render.fill(data, from_f32),
                on_error("speaker"),
                None,
            )
            .map_err(|err| format!("the speaker could not be opened: {err}"))
    }

    /// The default device of a direction, if the machine has a device at all.
    /// CoreAudio hands back a default with no device behind it where the
    /// machine has none (a Mac mini has no microphone), and the error that
    /// opening it gives means nothing to a person; `listed` says whether the
    /// host lists any device of the direction, and is asked on a Mac alone.
    /// ALSA's default is a virtual device that need not be among those it
    /// lists, so there it stands; and a list that could not be had does not
    /// say no.
    fn default_of(
        default: Option<cpal::Device>,
        listed: impl FnOnce() -> bool,
    ) -> Option<cpal::Device> {
        #[cfg(target_os = "macos")]
        {
            default.filter(|_| listed())
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = listed;
            default
        }
    }

    /// The default speaker's stream, not started, and how it is named.
    fn speaker(queue: Arc<Mutex<Queue>>) -> Result<(String, cpal::Stream), String> {
        let host = cpal::default_host();
        let output = default_of(host.default_output_device(), || {
            host.output_devices()
                .map_or(true, |mut all| all.next().is_some())
        })
        .ok_or("this machine has no default speaker")?;
        let config = output
            .default_output_config()
            .map_err(|err| format!("the speaker's format could not be read: {err}"))?;
        let stream = match config.sample_format() {
            SampleFormat::F32 => output_stream::<f32>(&output, &config, |s| s, queue)?,
            SampleFormat::I16 => output_stream::<i16>(&output, &config, to_i16, queue)?,
            other => {
                return Err(format!(
                    "the speaker's sample format ({other}) is not handled"
                ));
            }
        };
        Ok((describe(&output, &config), stream))
    }

    /// The microphones the host lists, in its order, and the devices behind
    /// them in the same order. A device with no id cannot be kept, so it is
    /// not offered. Nothing is opened ([`Input`] says why), and a host that
    /// cannot be asked lists none.
    fn listing(host: &cpal::Host) -> (Vec<Input>, Vec<cpal::Device>) {
        match host.input_devices() {
            Ok(all) => listing_of(all),
            Err(_) => (Vec::new(), Vec::new()),
        }
    }

    /// `devices` as [`listing`] has them.
    pub(super) fn listing_of(
        devices: impl Iterator<Item = cpal::Device>,
    ) -> (Vec<Input>, Vec<cpal::Device>) {
        devices
            .filter_map(|device| {
                let id = device.id().ok()?.to_string();
                let description = description_of(&device);
                Some((Input { id, description }, device))
            })
            .unzip()
    }

    pub(super) fn inputs() -> Vec<Input> {
        listing(&cpal::default_host()).0
    }

    /// The microphone a session opens for `kept`: the listed one with that
    /// id, and the default where none is kept or it is not listed ([`pick`]).
    /// The fall back is logged, since it is the one thing a person would not
    /// otherwise see: the session names the microphone that stands, and not
    /// the one that was asked for.
    pub(super) fn input_device(host: &cpal::Host, kept: &str) -> Option<cpal::Device> {
        if !kept.is_empty() {
            let (inputs, mut devices) = listing(host);
            if let Some(at) = pick(&inputs, kept) {
                return Some(devices.swap_remove(at));
            }
            say(&format!(
                "voice microphone: {kept} is not listed, so the default opens"
            ));
        }
        default_of(host.default_input_device(), || {
            host.input_devices()
                .map_or(true, |mut all| all.next().is_some())
        })
    }

    /// The microphone's stream, not started, and how it is named: the
    /// microphone `kept` names, or the default's.
    fn microphone(
        sink: impl FnMut(Vec<i16>) + Send + 'static,
        kept: &str,
    ) -> Result<(String, cpal::Stream), String> {
        let host = cpal::default_host();
        let input = input_device(&host, kept).ok_or("this machine has no default microphone")?;
        let config = input
            .default_input_config()
            .map_err(|err| format!("the microphone's format could not be read: {err}"))?;
        let stream = match config.sample_format() {
            SampleFormat::F32 => input_stream::<f32>(&input, &config, |s| s, sink)?,
            SampleFormat::I16 => input_stream::<i16>(&input, &config, to_f32, sink)?,
            other => {
                return Err(format!(
                    "the microphone's sample format ({other}) is not handled"
                ));
            }
        };
        Ok((describe(&input, &config), stream))
    }

    /// Both devices' streams, started, and how each is named. The speaker
    /// stream is built first; the consent check outside this function applies
    /// only to the microphone, the chosen one as the default.
    fn build(
        sink: impl FnMut(Vec<i16>) + Send + 'static,
        queue: Arc<Mutex<Queue>>,
        kept: &str,
    ) -> Result<(String, String, cpal::Stream, cpal::Stream), String> {
        let (output, out_stream) = speaker(queue)?;
        let (input, in_stream) = microphone(sink, kept)?;
        out_stream
            .play()
            .map_err(|err| format!("the speaker would not start: {err}"))?;
        in_stream
            .play()
            .map_err(|err| trouble("the microphone would not start", &err))?;
        Ok((input, output, in_stream, out_stream))
    }

    /// [`super::open`] where there are devices: both are built on a thread of
    /// their own, which holds them until the handle is dropped.
    pub(super) fn open(
        sink: impl FnMut(Vec<i16>) + Send + 'static,
        speaker: Arc<Mutex<Queue>>,
        microphone: &str,
    ) -> Result<Devices, String> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let kept = microphone.to_string();
        let thread = thread::spawn(move || match build(sink, speaker, &kept) {
            Ok((input, output, in_stream, out_stream)) => {
                let _ = ready_tx.send(Ok((input, output)));
                // The streams are held here until the handle is dropped,
                // which closes this channel: they are not `Send`.
                let _ = stop_rx.recv();
                drop(in_stream);
                drop(out_stream);
            }
            Err(why) => {
                let _ = ready_tx.send(Err(why));
            }
        });
        match ready_rx.recv() {
            Ok(Ok((input, output))) => {
                say(&format!(
                    "voice devices: microphone: {input}; speaker: {output}"
                ));
                Ok(Devices {
                    input,
                    output,
                    stop: Some(stop_tx),
                    thread: Some(thread),
                })
            }
            Ok(Err(why)) => {
                let _ = thread.join();
                Err(why)
            }
            Err(_) => Err("the audio thread ended before it opened a device".to_string()),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::f32::consts::PI;
        use std::time::{Duration, Instant};

        /// The system's no is said as one wherever cpal reports it, with
        /// where it is changed; any other trouble keeps cpal's own words.
        #[test]
        fn the_systems_no_to_the_microphone_is_said_as_one_and_other_trouble_in_cpals_words() {
            let no = cpal::Error::with_message(cpal::ErrorKind::PermissionDenied, "Unauthorized");
            assert_eq!(
                trouble("the microphone would not start", &no),
                refused(cfg!(target_os = "macos"))
            );
            let gone = cpal::Error::with_message(cpal::ErrorKind::DeviceNotAvailable, "gone");
            assert_eq!(
                trouble("the microphone would not start", &gone),
                "the microphone would not start: gone"
            );
        }

        /// The real default speaker takes the queue at the pace of its own
        /// clock, and a drop empties it. The tone is speech to the queue
        /// (-47 dBFS, just above its floor) and as good as silent in the room,
        /// and no microphone is opened, so a Mac asks for no consent. A
        /// person's to run: `cargo test audio -- --ignored --nocapture`.
        #[test]
        #[ignore = "opens the real speaker, with a tone near silence"]
        fn the_real_speaker_takes_the_queue_at_the_pace_of_its_clock_and_a_drop_empties_it() {
            let queue = Arc::new(Mutex::new(Queue::default()));
            let (name, stream) = speaker(queue.clone()).expect("the default speaker opens");
            println!("speaker: {name}");
            stream.play().expect("the speaker starts");
            let tone: Vec<i16> = (0..2 * RATE as usize)
                .map(|i| (200.0 * (2.0 * PI * 440.0 * i as f32 / RATE as f32).sin()) as i16)
                .collect();
            assert!(queue.lock().unwrap().push(tone, Instant::now()));
            std::thread::sleep(Duration::from_millis(500));
            let queued = queue.lock().unwrap().queued_ms();
            assert!(
                queued.abs_diff(1500) <= 250,
                "{queued} ms of 2000 are queued after 500 ms of the clock"
            );
            assert_eq!(queue.lock().unwrap().drop_queued(Instant::now()), queued);
            std::thread::sleep(Duration::from_millis(100));
            assert_eq!(queue.lock().unwrap().queued_ms(), 0);
            drop(stream);
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    //! The Mac's consent to the microphone, read from AVFoundation. The status
    //! query never asks the person anything. When it is not determined, the
    //! system prompt is requested here and its callback is awaited before the
    //! microphone opens.

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_av_foundation::{
        AVAuthorizationStatus, AVCaptureDevice, AVMediaType, AVMediaTypeAudio,
    };

    use super::{Consent, resolve_consent};

    /// This app's status for audio; for a program run from a terminal (`cargo
    /// run`), the terminal's, which answers for it. A new request is awaited
    /// on the device-opening thread, never the window's thread.
    pub(super) fn consent() -> Consent {
        let Some(audio) = (unsafe { AVMediaTypeAudio }) else {
            return Consent::NotDetermined;
        };
        resolve_consent(read(audio), || request_access(audio))
    }

    /// Reads AVFoundation's status without prompting. An unrecognized status
    /// is not a refusal.
    pub(super) fn read(audio: &AVMediaType) -> Option<Consent> {
        // SAFETY: the media type is AVMediaTypeAudio, one of the two the
        // method accepts; it is a status query and does not prompt.
        let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) };
        consent_of(status)
    }

    /// Asks through AVFoundation and waits for the person's decision. The
    /// caller is already on a background thread, so this does not block the
    /// window while the system displays its prompt.
    fn request_access(audio: &AVMediaType) -> bool {
        let (answer, decided) = std::sync::mpsc::channel();
        let completion = RcBlock::new(move |granted: Bool| {
            let _ = answer.send(granted.as_bool());
        });
        // SAFETY: `audio` is the framework's AVMediaTypeAudio constant, one
        // of the two supported media types, and the retained block matches
        // the method's BOOL completion result.
        unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &completion) }
        decided.recv().unwrap_or(false)
    }

    /// A status the table does not name (a later macOS may add one) is no
    /// refusal: it reads as unknown and opens as it did before the read.
    pub(super) fn consent_of(status: AVAuthorizationStatus) -> Option<Consent> {
        Some(match status {
            AVAuthorizationStatus::NotDetermined => Consent::NotDetermined,
            AVAuthorizationStatus::Restricted => Consent::Restricted,
            AVAuthorizationStatus::Denied => Consent::Denied,
            AVAuthorizationStatus::Authorized => Consent::Authorized,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;
    use std::time::{Duration, Instant};

    /// `seconds` of a tone of half scale, as a device gives it: `channels`
    /// interleaved, alike, at `rate`.
    fn tone(hz: f32, rate: u32, channels: usize, seconds: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize)
            .flat_map(|i| {
                let v = 0.5 * (2.0 * PI * hz * i as f32 / rate as f32).sin();
                std::iter::repeat_n(v, channels)
            })
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    fn db(x: f32) -> f32 {
        20.0 * x.log10()
    }

    fn rms_db(pcm: &[i16]) -> f32 {
        db(rms(&pcm.iter().map(|&s| to_f32(s)).collect::<Vec<_>>()))
    }

    #[test]
    fn a_resampler_keeps_a_tone_and_the_count() {
        for from in [48_000u32, 44_100, 16_000, 24_000] {
            let mut resampler = Resampler::new(from, RATE);
            let mut out = Vec::new();
            // Fed in uneven pieces, as a device calls back.
            for piece in tone(1000.0, from, 1, 1.0).chunks(333) {
                resampler.process(piece, &mut out);
            }
            assert!(
                out.len().abs_diff(RATE as usize) <= 3,
                "{from}: {} samples",
                out.len()
            );
            let level = rms(&out[100..out.len() - 100]);
            assert!((level - 0.3535).abs() < 0.04, "{from}: level {level}");
        }
    }

    #[test]
    fn a_resampler_goes_up_by_interpolating_and_a_constant_stays_one() {
        let mut resampler = Resampler::new(24_000, 48_000);
        let mut out = Vec::new();
        resampler.process(&[0.0, 1.0, 2.0, 3.0], &mut out);
        assert_eq!(out, [0.0, 0.5, 1.0, 1.5, 2.0, 2.5]);
        let mut resampler = Resampler::new(44_100, RATE);
        let mut out = Vec::new();
        resampler.process(&[0.25; 4410], &mut out);
        assert!(out.iter().all(|v| (v - 0.25).abs() < 1e-5));
    }

    #[test]
    fn a_microphone_becomes_frames_of_mono_24k_whatever_it_speaks() {
        // The default ALSA device of runs 3 to 6 (48000 Hz, two channels), a
        // Mac's built-in microphone, a headset at 44.1 kHz, an interface at
        // 96 kHz, a narrow 16 kHz one, and the socket's own rate.
        for (rate, channels) in [
            (48_000, 2),
            (48_000, 1),
            (44_100, 1),
            (96_000, 2),
            (16_000, 1),
            (24_000, 1),
        ] {
            let mut frames: Vec<Vec<i16>> = Vec::new();
            let mut capture = Capture::new(channels, rate, |frame| frames.push(frame));
            // A second of tone, as the device calls back: pieces of 333
            // frames, which no multiple of the frame divides.
            for piece in tone(1000.0, rate, channels, 1.0).chunks(333 * channels) {
                capture.feed(piece, |s| s);
            }
            drop(capture);
            // Fifty frames of 20 ms to the second: the socket's rate.
            assert!(
                frames.len().abs_diff(50) <= 1,
                "{rate} Hz x{channels}: {} frames",
                frames.len()
            );
            assert!(frames.iter().all(|frame| frame.len() == CHUNK));
            let level = rms_db(&frames[10]);
            assert!(
                (level + 9.0).abs() < 1.5,
                "{rate} Hz x{channels}: {level} dBFS for a half-scale sine"
            );
        }

        // A device of i16 samples is the same through another conversion, and
        // what is past full scale is held there.
        let mut frames: Vec<Vec<i16>> = Vec::new();
        let mut capture = Capture::new(1, 24_000, |frame| frames.push(frame));
        capture.feed(&vec![16384i16; 4800], to_f32);
        capture.feed(&[2.0f32; 1000], |s| s);
        drop(capture);
        assert_eq!(frames.len(), 12);
        assert_eq!(frames[0][5], 16384);
        assert_eq!(frames[11][5], 32767);
        assert_eq!(to_i16(-2.0), -32767);

        // One channel of several is not lost: the channels are averaged.
        let mut frames: Vec<Vec<i16>> = Vec::new();
        let mut capture = Capture::new(2, 24_000, |frame| frames.push(frame));
        let one_side: Vec<f32> = (0..CHUNK + 20).flat_map(|_| [0.5, 0.0]).collect();
        capture.feed(&one_side, |s| s);
        drop(capture);
        assert_eq!(frames[0][3], to_i16(0.25));
    }

    /// A stand-in speaker: the speaker's callbacks, `calls` of `frames`
    /// frames each in `channels`, as the device would make them.
    fn heard(render: &mut Render, calls: usize, frames: usize, channels: usize) -> Vec<f32> {
        let mut heard = Vec::new();
        for _ in 0..calls {
            let mut buffer = vec![0f32; frames * channels];
            render.fill(&mut buffer, |s| s);
            heard.extend(buffer);
        }
        heard
    }

    #[test]
    fn the_queue_comes_out_at_the_speakers_rate_in_its_channels_and_a_drop_is_silence() {
        let began = Instant::now();
        let at = |ms: u64| began + Duration::from_millis(ms);
        let queue = Arc::new(Mutex::new(Queue::default()));
        let mut render = Render::new(2, 48_000, queue.clone());
        let speech: Vec<i16> = (0..24_000)
            .map(|i| (12000.0 * (2.0 * PI * 1000.0 * i as f32 / 24_000.0).sin()) as i16)
            .collect();
        assert!(locked(&queue).push(speech.clone(), at(0)));

        // 20 callbacks of 512 frames at 48 kHz are 213 ms of the speaker's
        // time: both channels alike, the tone's level kept, and the queue
        // taken from at the pace of the speaker, not faster.
        let out = heard(&mut render, 20, 512, 2);
        assert!(out.chunks(2).all(|frame| frame[0] == frame[1]));
        let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
        let level = rms(&left[200..]);
        assert!((level - 0.2543).abs() < 0.03, "level {level}");
        let taken_ms = 1000 - locked(&queue).queued_ms();
        assert!(
            taken_ms.abs_diff(213) <= 12,
            "{taken_ms} ms of the queue went in 213 ms"
        );

        // Dropped, the next callback is silence at once, though the speaker
        // held audio of its own; what the server had already sent is thrown
        // away, and what comes after the holdoff is heard.
        locked(&queue).drop_queued(at(1000));
        // The resampler may already hold more than a small device callback.
        // It must still let go of those samples on the very next callback.
        let mut buffer = vec![1f32; 40 * 2];
        render.fill(&mut buffer, |s| s);
        assert!(buffer.iter().all(|v| *v == 0.0), "silence after a drop");
        let mut buffer = vec![1f32; 1024];
        render.fill(&mut buffer, |s| s);
        assert!(buffer.iter().all(|v| *v == 0.0), "silence after a drop");
        assert!(!locked(&queue).push(speech.clone(), at(1100)));
        assert!(heard(&mut render, 4, 512, 2).iter().all(|v| *v == 0.0));
        assert!(locked(&queue).push(speech[..4800].to_vec(), at(1300)));
        let out = heard(&mut render, 4, 512, 2);
        assert!(out.iter().any(|v| v.abs() > 0.1), "the voice again");

        // A speaker that takes i16 in one channel at 44.1 kHz is the same
        // through another conversion, and a queue with nothing in it is
        // silence.
        let queue = Arc::new(Mutex::new(Queue::default()));
        let mut render = Render::new(1, 44_100, queue.clone());
        let mut buffer = vec![5i16; 441];
        render.fill(&mut buffer, to_i16);
        assert!(buffer.iter().all(|s| *s == 0));
        assert!(locked(&queue).push(speech, at(0)));
        let mut buffer = vec![0i16; 4410];
        render.fill(&mut buffer, to_i16);
        let level = db(rms(&buffer[200..]
            .iter()
            .map(|&s| to_f32(s))
            .collect::<Vec<_>>()));
        assert!(
            (level + 11.9).abs() < 1.5,
            "{level} dBFS for 12000 of 32768"
        );
    }

    #[test]
    fn devices_are_named_so_that_a_person_can_tell_them_apart() {
        // A Mac mini's speakers and the monitor's, as the harness named them
        // on CoreAudio: the id is the device's UID, which says the real
        // device where the name is generic.
        assert_eq!(
            named(
                "Mac mini Speakers",
                Some("coreaudio:BuiltInSpeakerDevice"),
                48_000,
                2,
                "f32"
            ),
            "Mac mini Speakers, id coreaudio:BuiltInSpeakerDevice (48000 Hz, 2 ch, f32)"
        );
        assert_eq!(
            named(
                "MacBook Pro Microphone",
                Some("coreaudio:BuiltInMicrophoneDevice"),
                48_000,
                1,
                "f32"
            ),
            "MacBook Pro Microphone, id coreaudio:BuiltInMicrophoneDevice (48000 Hz, 1 ch, f32)"
        );
        // ALSA's default on a Linux desktop, which runs 3 to 6 sat on: all
        // cpal knows of it is that it is the default.
        assert_eq!(
            named(
                "Default Audio Device",
                Some("alsa:default"),
                48_000,
                2,
                "f32"
            ),
            "Default Audio Device, id alsa:default (48000 Hz, 2 ch, f32)"
        );
        // A card of ALSA's own, named by its first line of description.
        assert_eq!(
            named(
                "HDA Intel PCH, ALC3246 Analog",
                Some("alsa:sysdefault:CARD=PCH"),
                44_100,
                2,
                "i16"
            ),
            "HDA Intel PCH, ALC3246 Analog, id alsa:sysdefault:CARD=PCH (44100 Hz, 2 ch, i16)"
        );
        // A host that gives no id is still named.
        assert_eq!(
            named("an unnamed device", None, 16_000, 1, "i16"),
            "an unnamed device (16000 Hz, 1 ch, i16)"
        );
    }

    fn input(id: &str) -> Input {
        Input {
            id: id.to_string(),
            description: "a microphone".to_string(),
        }
    }

    /// Which microphone a session opens for the id the settings keep, with no
    /// device to open: the listed one that has the id, else the default.
    #[test]
    fn the_microphone_kept_is_the_one_opened_and_the_default_where_none_matches() {
        let listed = [
            input("coreaudio:BuiltInMicrophoneDevice"),
            input("coreaudio:AppleUSBAudioEngine:Blue:Yeti:1234"),
            input("coreaudio:AppleUSBAudioEngine:Blue:Yeti:5678"),
        ];
        // Nothing kept is the system's default, whatever is listed.
        assert_eq!(pick(&listed, ""), None);
        // A kept id picks its microphone wherever it stands in the list, and
        // two alike by name are told apart by the id.
        for (kept, at) in [
            ("coreaudio:BuiltInMicrophoneDevice", 0),
            ("coreaudio:AppleUSBAudioEngine:Blue:Yeti:1234", 1),
            ("coreaudio:AppleUSBAudioEngine:Blue:Yeti:5678", 2),
        ] {
            assert_eq!(pick(&listed, kept), Some(at), "{kept}");
        }
        // One the host no longer lists (unplugged, or named otherwise now)
        // is the default's, and so is any near miss: the id is the whole of
        // the name, as the host wrote it.
        for gone in [
            "coreaudio:AppleUSBAudioEngine:Blue:Yeti:9999",
            "coreaudio:BuiltInMicrophone",
            "coreaudio:builtinmicrophonedevice",
            "BuiltInMicrophoneDevice",
            " coreaudio:BuiltInMicrophoneDevice",
        ] {
            assert_eq!(pick(&listed, gone), None, "{gone}");
        }
        // A machine with no microphone listed opens the default, if it has
        // one, whatever is kept.
        assert_eq!(pick(&[], "coreaudio:BuiltInMicrophoneDevice"), None);
        assert_eq!(pick(&[], ""), None);
    }

    #[test]
    fn a_refused_microphone_says_where_it_is_changed() {
        // A Mac keeps its consent in System Settings, which the words name.
        let mac = refused(true);
        for place in ["System Settings", "Privacy & Security", "Microphone"] {
            assert!(mac.contains(place), "{mac}");
        }
        // Elsewhere there is no such place to name, and none is.
        let elsewhere = refused(false);
        assert!(!elsewhere.contains("System Settings"), "{elsewhere}");
        assert!(elsewhere.contains("refused"), "{elsewhere}");

        // The Mac's consent decides it: a no, the person's or a policy's, is
        // refused in those words, and a yes and a question that will be asked
        // before devices open are not.
        for (consent, said) in [
            (Consent::Denied, Err(mac.clone())),
            (Consent::Restricted, Err(mac.clone())),
            (Consent::Authorized, Ok(())),
            (Consent::NotDetermined, Ok(())),
        ] {
            assert_eq!(allowed(consent), said, "{consent:?}");
        }

        // An unanswered status asks once and resolves to the callback's
        // answer. Known statuses and unknown future statuses never ask.
        for (consent, answer, expected, asks) in [
            (
                Some(Consent::NotDetermined),
                true,
                Consent::Authorized,
                true,
            ),
            (Some(Consent::NotDetermined), false, Consent::Denied, true),
            (Some(Consent::Authorized), false, Consent::Authorized, false),
            (Some(Consent::Denied), true, Consent::Denied, false),
            (Some(Consent::Restricted), true, Consent::Restricted, false),
            (None, true, Consent::NotDetermined, false),
        ] {
            let asked = std::cell::Cell::new(false);
            assert_eq!(
                resolve_consent(consent, || {
                    asked.set(true);
                    answer
                }),
                expected
            );
            assert_eq!(asked.get(), asks, "{consent:?}");
        }
    }

    /// The consent is read before the devices open, so that a microphone
    /// already refused (including at the system prompt) is not opened, and
    /// again after, in case consent changes while the devices open. An open
    /// that fails says its own words.
    #[test]
    fn the_consent_is_read_before_the_devices_open_and_after_and_a_no_closes_them() {
        use std::cell::Cell;

        /// Devices that count themselves while they stand.
        struct Standing<'a>(&'a Cell<u32>);

        impl Drop for Standing<'_> {
            fn drop(&mut self) {
                self.0.set(self.0.get() - 1);
            }
        }

        let no = refused(true);
        // What the two reads say, whether the devices were opened at all, and
        // what came of it.
        for (before, after, opened, said) in [
            // Given: opened, and they stand.
            (Consent::Authorized, Consent::Authorized, true, Ok(())),
            // An unknown or missing status stays open as before.
            (Consent::NotDetermined, Consent::NotDetermined, true, Ok(())),
            // Consent changed while opening: the devices are dropped.
            (Consent::Authorized, Consent::Denied, true, Err(no.clone())),
            (
                Consent::Authorized,
                Consent::Restricted,
                true,
                Err(no.clone()),
            ),
            // Refused already, including at the prompt: devices are not opened.
            (Consent::Denied, Consent::Denied, false, Err(no.clone())),
            (
                Consent::Restricted,
                Consent::Restricted,
                false,
                Err(no.clone()),
            ),
        ] {
            let case = format!("{before:?}, then {after:?}");
            let (opens, standing, reads) = (Cell::new(0), Cell::new(0), Cell::new(0));
            let got = consenting(
                || {
                    reads.set(reads.get() + 1);
                    if reads.get() == 1 { before } else { after }
                },
                || {
                    opens.set(opens.get() + 1);
                    standing.set(standing.get() + 1);
                    Ok(Standing(&standing))
                },
            );
            assert_eq!(opens.get(), u32::from(opened), "{case}");
            assert_eq!(reads.get(), if opened { 2 } else { 1 }, "{case}");
            assert_eq!(got.as_ref().err(), said.as_ref().err(), "{case}");
            assert_eq!(standing.get(), u32::from(said.is_ok()), "{case}");
            drop(got);
            assert_eq!(standing.get(), 0, "{case}");
        }

        // A microphone that is not there is said as it is, and the consent is
        // not read again.
        let reads = Cell::new(0);
        let none = "this machine has no default microphone".to_string();
        let got = consenting(
            || {
                reads.set(reads.get() + 1);
                Consent::Authorized
            },
            || Err::<(), _>(none.clone()),
        );
        assert_eq!(got, Err(none));
        assert_eq!(reads.get(), 1);
    }

    /// AVFoundation's four statuses, and one a later macOS may add, read as the
    /// consent each means; and the real status query answers on this machine,
    /// whatever its consent is, with no prompt and no exception (the framework
    /// throws on a media type it does not take). A new permission request needs
    /// a person at a Mac and is not tried here.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_macs_authorization_status_is_read_as_the_consent_it_means() {
        use objc2_av_foundation::{AVAuthorizationStatus, AVMediaTypeAudio};

        for (status, consent) in [
            (
                AVAuthorizationStatus::NotDetermined,
                Some(Consent::NotDetermined),
            ),
            (AVAuthorizationStatus::Restricted, Some(Consent::Restricted)),
            (AVAuthorizationStatus::Denied, Some(Consent::Denied)),
            (AVAuthorizationStatus::Authorized, Some(Consent::Authorized)),
            // No refusal and no prompt: it opens as it did before the read.
            (AVAuthorizationStatus(99), None),
        ] {
            assert_eq!(mac::consent_of(status), consent, "{status:?}");
        }
        let audio = unsafe { AVMediaTypeAudio }.expect("audio media type is present");
        let here = mac::read(audio);
        println!("consent here: {here:?}");
    }

    #[test]
    fn the_audio_path_moves_frames_and_knows_no_provider() {
        // What a provider's session needs stays in voice.rs: the devices
        // here take mono PCM16 in and give it out, and name no event, key or
        // delegation of any. This reads the module above its tests, which
        // name these words to look for them.
        // A Windows checkout has CRLF line ends, which the cut must not miss.
        let source = include_str!("audio.rs").replace("\r\n", "\n");
        let (source, _) = source
            .split_once("\n#[cfg(test)]\nmod tests {")
            .expect("the module's tests are where the cut looks for them");
        for word in [
            "OpenAI",
            "GPT",
            "delegat",
            "commentary",
            "input_audio",
            "output_audio",
            "Bearer",
        ] {
            assert!(!source.contains(word), "{word} is named");
        }
    }

    /// The real default devices of the machine: opened for a moment and named,
    /// to see on a Mac or a Linux desktop what a session would report. It
    /// opens the microphone (the Mac asks for its consent), so it is a
    /// person's to run: `cargo test audio -- --ignored --nocapture`.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    #[ignore = "opens the real microphone and speaker"]
    fn the_real_devices_open_and_name_themselves() {
        let (tx, frames) = mpsc::channel();
        let devices = open(
            move |frame| {
                let _ = tx.send(frame);
            },
            Arc::new(Mutex::new(Queue::default())),
            "",
        )
        .expect("the default microphone and speaker open");
        println!("microphone: {}", devices.input);
        println!("speaker: {}", devices.output);
        // The microphone's first frame is the socket's: 20 ms of mono at 24 kHz.
        let frame = frames
            .recv_timeout(Duration::from_secs(5))
            .expect("a frame from the microphone in five seconds");
        assert_eq!(frame.len(), CHUNK);
    }

    /// The machine's real microphones, listed and found by the id each is
    /// kept under, and the default found where the id is not listed. Nothing
    /// is opened, so a Mac asks for no consent and no microphone is heard;
    /// it is a person's to run for the names it prints:
    /// `cargo test audio -- --ignored --nocapture`. A machine with no
    /// microphone (a Mac mini) lists none, and the ids and names of all its
    /// devices stand in their place, listed the way the microphones are.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    #[ignore = "lists the machine's real microphones"]
    fn the_real_microphones_are_listed_and_found_by_the_id_they_are_kept_under() {
        use cpal::traits::{DeviceTrait, HostTrait};

        let host = cpal::default_host();
        let listed = inputs();
        println!(
            "host: {} devices, {} take input, default input {:?}",
            host.devices().map_or(0, |all| all.count()),
            host.input_devices().map_or(0, |all| all.count()),
            host.default_input_device().and_then(|d| d.id().ok()),
        );
        for input in &listed {
            println!("input: {}  |  {}", input.id, input.description);
            let found = device::input_device(&host, &input.id).expect("a listed microphone");
            assert_eq!(found.id().unwrap().to_string(), input.id);
        }
        let default = host.default_input_device().and_then(|d| d.id().ok());
        let gone = device::input_device(&host, "alsa:no such microphone");
        if default.is_some() && !listed.is_empty() {
            assert_eq!(gone.and_then(|d| d.id().ok()), default);
        }
        // Whatever the machine's devices are, each is listed by the id it is
        // kept under, which the rule finds at its own place and no other.
        let (all, _) = device::listing_of(host.devices().expect("the host's devices"));
        for (at, device) in all.iter().enumerate() {
            println!("device: {}  |  {}", device.id, device.description);
            assert_eq!(pick(&all, &device.id), Some(at));
        }
    }
}
