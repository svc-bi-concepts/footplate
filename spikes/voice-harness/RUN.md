# voice-harness: how to run it

A throwaway program for bead footplate-4hd3 of footplate's voice design
(`docs/design/2026-10-31-footplate-vqze-voice.md`): your microphone to
GPT-Live (`gpt-live-1`) over a WebSocket written by hand, its speech played as
it comes, and the yard's answer typed by you and returned by
`session.commentary.append` to be spoken in the model's own words. It logs what
a person reads back afterwards. It is not footplate's product code. Live
sessions send audio, transcripts and the project key only to
`api.openai.com`; `--url` can change the destination for a keyless `--probe`.

## The command

On the Mac, from a footplate checkout (or the unpacked archive from the bead's
note), with a microphone and, best, a headset:

    cd spikes/voice-harness
    cargo run --release --locked

The key is your own OpenAI project's. Type it into the masked field in
footplate's Settings, Voice section. footplate keeps it in `voice.key` beside
its `settings.json`, and the harness reads it from there. The other way is the
environment, for when there is no footplate to type it in, or when one run
should use another key:

    FOOTPLATE_OPENAI_KEY=sk-... cargo run --release --locked

`--locked` builds what the committed `Cargo.lock` names; the first build takes a
minute or two (cpal and its Apple bindings). To build once and run many times:
`cargo build --release --locked`, then `./target/release/voice-harness [options]`.

- When `FOOTPLATE_OPENAI_KEY` has a non-empty value after trimming, it is the
  key and the file is not read. An empty or whitespace-only value falls back
  to the Settings key. A variable left in your shell therefore wins over what
  you typed in Settings. Run `unset FOOTPLATE_OPENAI_KEY` to use the Settings
  key.
- Otherwise the key is in `voice.key` beside footplate's `settings.json`, where
  footplate keeps its settings: `~/Library/Application Support/footplate/` on a
  Mac, `~/.config/footplate/` on Linux, or `$XDG_CONFIG_HOME/footplate/` when
  that is an absolute path.
- The harness refuses what footplate refuses, in words: a file others can read
  (`chmod 600` it, or type the key again in Settings), a file that is not a
  regular file, a file without the key on its first line and footplate's marker
  under it (one made by hand), and a variable that is not one line. With no key
  at all it says to type one in Settings, Voice, or to set the variable.
- The key is never printed, never in the log, never in an error. A server's
  refusal has anything key-shaped taken out, and the harness's own words about
  the key's file never say what it holds. The log holds what you said and what
  the model said, since that is what it is for. It is written to
  `voice-harness-<time>.log` here, readable by you alone, and to your terminal.
- The first time, macOS asks whether your terminal may use the microphone:
  allow it. Where consent was refused macOS gives the program silence, not an
  error; the harness says so (`WARN`) after 3 s of exact silence.
- `/q` or Ctrl-C ends the session properly (`session.close`, then it waits for
  `session.closed`, which carries the final usage). A session costs $0.05 a
  minute; the log shows the cost.

Four checks to run first. They need no key, they open no microphone (so no
consent prompt), and they read no key file:

    cargo run --release --locked -- --rehearse       # a whole session against a stand-in, ~4 s
    cargo run --release --locked -- --devices        # every microphone and speaker, the defaults marked
    cargo run --release --locked -- --probe          # reaches api.openai.com with no key; expects a 401
    cargo run --release --locked -- --speaker-test   # one second of tone through the default speaker

`--speaker-test` is the one that makes a sound: 440 Hz for a second, a little
under the volume speech is played at, by the same playback path a session uses,
on the speaker named in its first line. If you hear it, the speaker path works,
and a session with no sound is the model's or the microphone's. If you do not,
no session will be heard: look at the device it names (System Settings, Sound,
Output), its volume and mute, and whether a headset is the default.
`--devices` names each device with the host's id for it (on a Mac its UID,
`coreaudio:BuiltInSpeakerDevice`), which tells the real device behind a generic
name, and the first line of a session's log (`DEVICES`) says the same.

## If a session gives no sound and no words

The first live run (a plain session, 33 s, on a Mac) was accepted and gave
nothing: no error, no transcript in either direction, no delegation, and the
level gate never fell quiet. What OpenAI's pages settle about that (read 1 Nov,
developers.openai.com/api/reference/resources/live/primary-websocket.md and
api/docs/guides/{live-conversations,voice-websockets}.md), and what they do not:

- **Nothing in `session.start` is missing.** The page's session has `model`,
  `audio` (`format`, `output.voice`), `client` (a WebRTC frontend's
  permissions), `delegation`, `input`, `instructions`, `store` and `triggers`,
  and no setting for transcripts: they come as
  `session.input_transcript.delta` and `session.output_transcript.delta`
  "for intervals that contain text". Instructions are optional ("Omitted or
  blank instructions use server defaults"). The harness waits for
  `session.started` before it sends audio and sends the microphone's audio
  without a break, silence included, as the page asks.
- **The model does not speak first.** "Startup instructions can improve
  greeting behavior, but do not guarantee that the model will speak." A plain
  session is silent until it hears you, or is given something to say.
- **The stream runs on through its silences.** The page does not say so; the
  run's own numbers do: 32660 ms of audio at the pace of the clock in a 33 s
  session in which the model said nothing was padding, not speech. The harness
  used to take every delta for the model speaking; it now sets audio below
  -50 dBFS (`SOUND_DB` in `src/audio.rs`, a guess) apart as padding, and the
  acknowledgement and interruption figures read speech only.
- **What only a live run can settle:** whether that padding is digital zeros or
  something quiet, whether the microphone's audio reached the model as speech
  (the first run's level gate says something loud was heard; no level says what),
  and whether the model hears you and answers. The levels below are for that.

What to run next, in this order, and stop at the first that does not do what it
says:

1. `--speaker-test` (above): do you hear the tone? No: fix the speaker first.
2. `--devices` (above): are the microphone and speaker the ones you mean? (A
   headset that is the default for one and not the other, or a monitor's
   speakers, are usual.)
3. A plain session of about a minute (`cargo run --release --locked`). When the
   `SESSION` lines show, **type a line and press Enter**, for instance
   `Say hello, and say that you can hear me.` With no delegation a typed line
   goes out as `session.commentary.append` with `delegation_id: null`, which is
   the page's own way to make the model speak ("Greet before the caller
   speaks"), and it needs no microphone. Then speak. Read the `SYNC` lines as
   below, and look for `SAID` (the model's words), `IN` (yours) and any `WARN`.
4. Send back the `voice-harness-*.log` it wrote, after reading it: it carries
   your words.

### Reading the levels

Every `SYNC` line (every 15 s) ends with two levels for the interval since the
last: `microphone this interval` and `model audio this interval`; the `SUMMARY`
has the same for the whole session, and typing `/status` prints them so far,
without waiting for the next `SYNC`. A level is `rms X dBFS, peak Y dBFS`: the
average loudness of every sample and the highest one, 0 being full scale. A
voice at a good microphone is around -30 to -20 rms with peaks above -15; a
quiet room is below -50. `no audio` means none arrived in the interval, and
`digital silence (every sample exactly 0)` means every sample was zero, which
nothing alive produces.

| Microphone says | It means | Do |
| --- | --- | --- |
| `digital silence` | The system gives the program zeros: consent refused (a `WARN` after 3 s says so), the input muted, or the wrong input | System Settings, Privacy & Security, Microphone: allow the terminal; check `DEVICES` |
| rms and peak within 2 or 3 dB, gate open the whole interval | A flat signal, not a voice: a DC offset, a hum, or a clipped input | Another input; a headset |
| rms above -35, gate open nearly all the interval | A noisy room, or you talking without a pause; the gate (-35) never falls quiet | `cargo run --release --locked -- --vad-db -25`; a quieter room; a headset |
| rms from -60 (an interval of room noise) to -30 (you spoke for part of it), peaks above -20 when you spoke, gate open only while you speak | A working microphone | If the model still says nothing, it is the next table |

| Model audio says | It means |
| --- | --- |
| `digital silence`, or nothing above the speech floor | The stream is padding and the model said nothing. With a working microphone and a typed line that got no speech either, send the log back: the model is not answering |
| low but not zero (rms -70, say), nothing above the floor | Padding with noise in it: the floor (`SOUND_DB`) may be wrong, or the model said nothing |
| speech-like (rms -30 to -20, peak -15 to -5), some ms above the floor | The model spoke. If you heard nothing, the speaker path is the trouble (`--speaker-test`) |

A model that speaks (words on `SAID` lines, ms above the floor) while no `IN`
line ever shows your words, with a microphone that reads as a voice, is the
model not hearing you: send the log back.

## What to do, and what each run settles

Speak to it. When it hands a request to the yard the log shows `DELEGATION` and
`TASK` (the words taken from what you said; the event carries none). Type the
yard's answer on a line and press Enter; the model speaks it. Any other line is
that answer; `/t [text]` sends a thinking append instead (empty: quiet),
`/status` prints the counts, `/q` ends.

1. **Delegation and the typed answer** (3 minutes). Run it plain. Say "What's
   blocking the release?" Expect `DELEGATION d1`, then a spoken "let me check"
   (`ACK-SPEECH`), and type: `Everything else has landed. The Windows zip is still
   building.` Expect `SPOKEN d1` and a `COMPARE` block: your text, the words
   spoken, how much of yours was kept and which words were said that you did not
   type (the design's "may state a yard fact never given").
2. **The acknowledgement and an append that cancels it.** The delay is 800 ms. By
   hand you cannot type inside it, so: `-- --auto-answer 300:"The Windows zip is
   still building."` answers each delegation 300 ms after it. Ask three
   different things. `ACK d1 ...` says whether the model spoke before the delay
   (`BEFORE the 800 ms delay`), after it, or whether the append `CANCELLED` it.
   Then `-- --quiet-on-delegation` (the guide's empty thinking append at once) and
   `-- --ack-delay-ms 0` for the default of the API.
3. **Interruption** (a headset: the level gate hears the model through
   speakers). Type a long answer (three sentences) and speak over the model's
   speech. `INTERRUPT` lines say how much speech was queued ahead, how late the
   server's last audio came, and how long until the speaker fell silent.
   Run it once as is (nothing dropped: the server must stop and the queue play
   out) and once with `-- --barge-in` (the queue is dropped when you are heard,
   and audio arriving in the next `--holdoff-ms`, 300, is thrown away too).
   The two `stopped` figures are the evidence. (The API has no event that says
   "interrupted"; the level gate here stands in for what footplate would have to
   do.)
4. **Five minutes** (caption against audio). `-- --stop-after 300` and talk with
   it, several delegations, for the five minutes. The `SUMMARY` ends with three
   drift lines (caption timeline minus wall clock, audio sent minus input caption
   end, output caption end minus audio played): first, last, range and a slope in
   ms per minute. A flat slope is "no drift". `SYNC` lines every 15 s show the
   raw numbers.
5. **Laptop speakers without a headset.** Run 4 again on the speakers. Lines
   tagged `ECHO?` are what the microphone heard that was the model's own words;
   note whether it interrupted itself or answered itself.

Send back the log files (`voice-harness-*.log`) on the next bead's note, after
reading them: they carry your words.

## The log's lines

    DEVICES    the microphone and speaker in use: name, the host's id, format
    START      what was asked of the session (delegation, delay, rate, store false)
    SESSION    started; the id; expires_at and the seconds it gives the session; then
               what the server resolved it as (audio format and rate, voice, delegation)
    WARN       the audio came back resolved as other than the PCM the harness sends and
               plays; or the microphone has given exact silence for 3 s
    IN         your words as the transcript grows (caption), with audio sent and played
    DELEGATION the delegation and its place on the session's timeline; TASK: its words
    ACK-SPEECH what the model said before the answer (the automatic acknowledgement)
    ANSWER     what was sent for a delegation, typed or --auto-answer; APPEND-OK: its acknowledgement
    SPOKEN     what the model then said; COMPARE: typed and spoken side by side
    SAID       what the model said that answers no delegation and is no acknowledgement
    SPEECH     the level gate heard you begin to speak, or stop
    INTERRUPT  you spoke over the model: queued ahead, dropped, the server's last audio, time to silence
    ACK        per delegation, at the end: spoke before the delay / cancelled / nothing
    SYNC       every 15 s: audio sent, received and played; the caption timeline against the
               clock; the microphone's level and the model's audio level for the interval
               (rms and peak in dBFS, `digital silence`, or `no audio`), with how long the
               level gate was open and how much of the model's audio was above the speech floor
    ECHO?      (on IN) the microphone heard the model's own words
    SUMMARY    counts, each direction's level over the whole session, the session's limit,
               the three drifts, each interruption
    EVENT/NOTE/ERROR  anything the harness did not expect, shortened; ERROR is the server's

A rehearsal's log, for the shape (all of it from the stand-in, none from OpenAI):

    [   0.231s] DELEGATION d1 id del_1 at offset 220 ms, target client (the event carries no task text)
    [   0.231s] TASK      d1 from the transcript: "What's blocking the release?"
    [   1.161s] ACK-SPEECH d1 [1126..1626 ms] "Let me check."
    [   1.161s] ANSWER    d1 typed, 929 ms after the delegation: "Everything else has landed. ..."
    [   1.439s] INTERRUPT you began to speak while the model was speaking; 1040 ms of speech was queued ahead, and 1040 ms of it was dropped at once (--barge-in)
    [   1.495s] SPOKEN    d1 [1155..3155 ms] "Everything else has landed, and the Windows zip is still building, due by four."
    [   1.496s] COMPARE   d1 ... 100% of the typed content words were kept; words spoken that were not typed: due, four
    [   2.245s] ACK       d1 spoke 905 ms after the delegation: at or after the 800 ms delay
    [   2.245s] ACK       d2 an append at 52 ms, inside the 800 ms delay, and no speech came first: CANCELLED

and two of its `SYNC` lines (the stand-in's stream runs on in silence between
its speech, as the real one seems to; the rehearsal's SYNC is every second):

    [   1.013s] SYNC      1 s in: audio sent 980 ms; received 1000 ms, played 1000 ms; caption timeline vs wall -1 ms; microphone this interval: digital silence (every sample exactly 0); model audio this interval: digital silence (every sample exactly 0)
    [   2.043s] SYNC      2 s in: audio sent 1980 ms; received 2480 ms, played 2000 ms; caption timeline vs wall -1 ms; microphone this interval: rms -25.2 dBFS, peak -14.7 dBFS (level gate open 420 of 1000 ms); model audio this interval: rms -24.2 dBFS, peak -20.8 dBFS (1340 of 1480 ms above the -50 dBFS speech floor)

## Options

`--help` lists them all: `--counterpart`, `--ack-delay-ms`, `--quiet-on-delegation`,
`--barge-in`, `--holdoff-ms`, `--vad-db` (the level gate, -35 dBFS), `--auto-answer
MS:TEXT`, `--stop-after SECS`, `--instructions FILE`, `--log FILE`, `--url`
(for `--probe`; live sessions are pinned to OpenAI's endpoint), and the keyless
`--rehearse`, `--devices [--play-silence]`, `--probe` and `--speaker-test`.

## What was and was not tried before this was handed over

Tried, with no key and no network beyond one keyless request: the framing and
the events against a stand-in socket on a loopback port (`cargo test --locked`),
a keyless `--probe` of `api.openai.com` (TLS and the upgrade request reach it;
it answers 401 `missing_authorization`), and, on a Mac mini's speaker and in
silence (`--devices --play-silence`), the harness's own playback path: the
callback's size (512 frames) and the device's latency (12 ms), the played count
against the clock, and the speaker going quiet 13 to 16 ms after a queue drop.
The microphone's conversions are unit-tested. Not tried, since the builder's
machine has no microphone and no key: a microphone opened, any real session, and
any sound heard.

The reading of the key from footplate's file came after, and was tried against
temporary directories only. `cargo test --locked` holds the order of the
variable and the file, a file written the way footplate's `keep_voice_key`
writes it, and each refusal. The built program also ran under `env -i` with a
home of its own. It gave the same refusals, found a key in the file, and ran
`--rehearse`, `--devices` and `--probe` (against a listener of its own) as
before, with a `voice.key` beside that any read would refuse. Not tried: a key
typed into a running footplate window, and a live session on a key read from
the file.

The first live run's silence was then looked into (bead footplate-3rbp), again
with no key, no microphone and no live session. Tried: OpenAI's pages read
against the `session.start` and first events the harness sends and reads (the
section above). `cargo test --locked` holds, against the stand-in (which now
runs its stream on in silence between its speech, as the real one seems to), the
levels of loud and of silent audio in both directions on the `SYNC` lines and in
the summary, the queue's setting padding apart from speech, the acknowledgement
and the interruption read from speech and not from padding, and what
`session.started` resolved. `--speaker-test` ran on a Mac mini's speaker under
`env -i` with a home of its own whose `voice.key` any read would refuse: it
named the speaker, the speaker's callback took the whole tone, and no file was
read, where the same environment in live mode was refused at the file. Not
tried: a microphone, a real session, and whether the tone was *heard*. So why
the first run gave nothing is not known; the next run's levels are meant to
show it.
