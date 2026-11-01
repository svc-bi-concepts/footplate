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

Three checks to run first. They need no key and no microphone, and they read no
key file:

    cargo run --release --locked -- --rehearse       # a whole session against a stand-in, ~3 s
    cargo run --release --locked -- --devices        # your microphone and speaker, what cpal offers
    cargo run --release --locked -- --probe          # reaches api.openai.com with no key; expects a 401

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

    START      what was asked of the session (delegation, delay, rate, store false)
    SESSION    started; the id; expires_at and the seconds it gives the session
    IN         your words as the transcript grows (caption), with audio sent and played
    DELEGATION the delegation and its place on the session's timeline; TASK: its words
    ACK-SPEECH what the model said before the answer (the automatic acknowledgement)
    ANSWER     what was sent for a delegation, typed or --auto-answer; APPEND-OK: its acknowledgement
    SPOKEN     what the model then said; COMPARE: typed and spoken side by side
    INTERRUPT  you spoke over the model: queued ahead, dropped, the server's last audio, time to silence
    ACK        per delegation, at the end: spoke before the delay / cancelled / nothing
    SYNC       every 15 s: audio sent, received and played; the caption timeline against the clock
    ECHO?      (on IN) the microphone heard the model's own words
    SUMMARY    counts, the session's limit, the three drifts, each interruption
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

## Options

`--help` lists them all: `--counterpart`, `--ack-delay-ms`, `--quiet-on-delegation`,
`--barge-in`, `--holdoff-ms`, `--vad-db` (the level gate, -35 dBFS), `--auto-answer
MS:TEXT`, `--stop-after SECS`, `--instructions FILE`, `--log FILE`, `--url`
(for `--probe`; live sessions are pinned to OpenAI's endpoint).

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
