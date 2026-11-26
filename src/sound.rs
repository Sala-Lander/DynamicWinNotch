//! Sounds. No audio files are embedded: the built-in tones are synthesised at start-up from the
//! small note tables below, so the app stays a single .exe. Playback is winmm `PlaySoundW` from
//! memory (async, nothing runs in the background).
//!
//! HOW TO CHANGE SOUNDS
//!   1. Easiest: drop your own `.wav` into `%APPDATA%\NotchIsland\sounds\` (tray menu ->
//!      "Open sounds folder"), named after the sound (see `Sound::stem`), then "Reload config".
//!      A file there always wins over the generated sound. 8/16-bit PCM, mono or stereo, up to 4 MB.
//!   2. Tweak the built-in tones: edit the note tables right below (`hz`, `at_ms`, `len_ms`, `gain`)
//!      or the three packs in `pack_style`.
//!   3. Add a brand-new sound: add a variant to `Sound` (+ `ALL`, `stem`), give it a recipe in
//!      `recipe`, and call `sounds.ui(..)` / `sounds.event(..)` where it should play.
//!
//! No OS calls except the two thin wrappers at the bottom, so everything else is unit-tested.

use crate::config::{sev, AlarmRepeat, AlarmTone, Config, SoundPack};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Every sound the island can make. The name is also the `.wav` file name you can override it with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    /// Click on a tab, button or setting.
    Tick,
    /// The island expands.
    Open,
    /// The island collapses.
    Close,
    /// A Windows notification arrives.
    Notify,
    /// Charger plugged / unplugged, battery low / full.
    Power,
    /// Something was copied to the clipboard.
    Copy,
    /// A Bluetooth device connects / disconnects.
    Bluetooth,
    /// The countdown timer ends (repeats per `alarm_repeat`).
    Alarm,
    /// A reminder comes due (plays once; falls back to `alarm.wav` if there is no `reminder.wav`).
    Reminder,
}

impl Sound {
    pub const ALL: [Sound; 9] = [
        Sound::Tick,
        Sound::Open,
        Sound::Close,
        Sound::Notify,
        Sound::Power,
        Sound::Copy,
        Sound::Bluetooth,
        Sound::Alarm,
        Sound::Reminder,
    ];

    /// File name (without `.wav`) of the user override for this sound.
    pub fn stem(self) -> &'static str {
        match self {
            Sound::Tick => "tick",
            Sound::Open => "open",
            Sound::Close => "close",
            Sound::Notify => "notify",
            Sound::Power => "power",
            Sound::Copy => "copy",
            Sound::Bluetooth => "bluetooth",
            Sound::Alarm => "alarm",
            Sound::Reminder => "reminder",
        }
    }

    fn idx(self) -> usize {
        self as usize
    }
}

// ------------------------------------------------------------------ note tables (edit me)

/// One tone: frequency in Hz, when it starts and how long it rings (ms), relative loudness.
#[derive(Clone, Copy, Debug)]
pub struct Note {
    pub hz: f32,
    pub at_ms: u32,
    pub len_ms: u32,
    pub gain: f32,
}

const fn n(hz: f32, at_ms: u32, len_ms: u32) -> Note {
    Note { hz, at_ms, len_ms, gain: 1.0 }
}

const fn ng(hz: f32, at_ms: u32, len_ms: u32, gain: f32) -> Note {
    Note { hz, at_ms, len_ms, gain }
}

const TICK: &[Note] = &[n(1300.0, 0, 35)];
const OPEN: &[Note] = &[n(660.0, 0, 70), n(990.0, 45, 110)]; // rising
const CLOSE: &[Note] = &[n(880.0, 0, 70), n(587.0, 45, 110)]; // falling
const NOTIFY: &[Note] = &[n(784.0, 0, 140), n(1175.0, 120, 240)];
const POWER: &[Note] = &[n(523.0, 0, 140), n(784.0, 120, 240)];
const COPY: &[Note] = &[n(1000.0, 0, 50), n(1500.0, 60, 70)];
const BLUETOOTH: &[Note] = &[n(659.0, 0, 140), n(880.0, 120, 220)];

// Alarm tones (also used for reminders).
const CHIME: &[Note] = &[n(523.0, 0, 380), n(659.0, 160, 380), n(784.0, 320, 380), n(1047.0, 480, 520)];
const BEEP: &[Note] = &[n(880.0, 0, 130), n(880.0, 200, 130), n(880.0, 400, 130)];
const BELL: &[Note] = &[n(880.0, 0, 1100), ng(1760.0, 0, 600, 0.4), ng(2217.0, 0, 350, 0.2)];

/// The note table for a sound (`tone` only matters for `Alarm` / `Reminder`).
pub fn recipe(sound: Sound, tone: AlarmTone) -> &'static [Note] {
    match sound {
        Sound::Tick => TICK,
        Sound::Open => OPEN,
        Sound::Close => CLOSE,
        Sound::Notify => NOTIFY,
        Sound::Power => POWER,
        Sound::Copy => COPY,
        Sound::Bluetooth => BLUETOOTH,
        Sound::Alarm | Sound::Reminder => match tone {
            AlarmTone::Beep => BEEP,
            AlarmTone::Bell => BELL,
            AlarmTone::Chime | AlarmTone::Off => CHIME,
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wave {
    Sine,
    Triangle,
    Square,
}

/// (waveform, loudness) of a pack. Square waves are harsh, hence the lower gain.
fn pack_style(p: SoundPack) -> (Wave, f32) {
    match p {
        SoundPack::Soft => (Wave::Sine, 1.0),
        SoundPack::Glass => (Wave::Triangle, 0.9),
        SoundPack::Retro => (Wave::Square, 0.35),
    }
}

/// Pause between repeats of the alarm.
const REPEAT_GAP_MS: u32 = 350;
/// How long `alarm_repeat = loop` keeps going without a click.
const LOOP_MS: u32 = 30_000;
/// Upper bound for a user file after repeating (bytes).
const MAX_WAV: usize = 8 * 1024 * 1024;
const MAX_USER_FILE: u64 = 4 * 1024 * 1024;

// -------------------------------------------------------------------------------- synthesis

pub const RATE: u32 = 22_050;
const ATTACK_S: f32 = 0.006;

fn osc(w: Wave, phase: f32) -> f32 {
    match w {
        Wave::Sine => (phase * std::f32::consts::TAU).sin(),
        Wave::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
        Wave::Square => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
    }
}

/// Mix a note table into mono 16-bit samples. `volume` is 0-100.
pub fn synth(notes: &[Note], pack: SoundPack, volume: u32) -> Vec<i16> {
    let (wave, pack_gain) = pack_style(pack);
    let amp = 0.55 * pack_gain * (volume.min(100) as f32 / 100.0);
    let end_ms = notes.iter().map(|x| x.at_ms + x.len_ms).max().unwrap_or(0) + 20;
    let total = (RATE as u64 * end_ms as u64 / 1000) as usize;
    let mut mix = vec![0f32; total];
    for note in notes {
        let start = (RATE as u64 * note.at_ms as u64 / 1000) as usize;
        let len = (RATE as u64 * note.len_ms as u64 / 1000) as usize;
        let step = note.hz / RATE as f32;
        let mut phase = 0.0f32;
        for i in 0..len {
            let Some(slot) = mix.get_mut(start + i) else { break };
            let t = i as f32 / RATE as f32;
            let x = i as f32 / len as f32;
            let env = (t / ATTACK_S).min(1.0) * (1.0 - x) * (1.0 - x);
            *slot += osc(wave, phase) * env * note.gain;
            phase += step;
            if phase >= 1.0 {
                phase -= 1.0;
            }
        }
    }
    // Several partials can add up past full scale: scale the whole clip down instead of clipping.
    let peak = mix.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let norm = if peak > 1.0 { 1.0 / peak } else { 1.0 };
    mix.iter().map(|&v| ((v * norm * amp).clamp(-1.0, 1.0) * 32767.0) as i16).collect()
}

fn wav_header(fmt: &[u8; 16], data_len: usize) -> Vec<u8> {
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(fmt);
    h.extend_from_slice(b"data");
    h.extend_from_slice(&(data_len as u32).to_le_bytes());
    h
}

/// A complete WAV file (mono, 16-bit) for these samples.
pub fn wav_from_samples(samples: &[i16]) -> Vec<u8> {
    let mut fmt = [0u8; 16];
    fmt[0..2].copy_from_slice(&1u16.to_le_bytes()); // PCM
    fmt[2..4].copy_from_slice(&1u16.to_le_bytes()); // mono
    fmt[4..8].copy_from_slice(&RATE.to_le_bytes());
    fmt[8..12].copy_from_slice(&(RATE * 2).to_le_bytes()); // byte rate
    fmt[12..14].copy_from_slice(&2u16.to_le_bytes()); // block align
    fmt[14..16].copy_from_slice(&16u16.to_le_bytes());
    let mut out = wav_header(&fmt, samples.len() * 2);
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

// ------------------------------------------------------------------------- WAV handling (user files)

struct Wav {
    channels: u16,
    rate: u32,
    bits: u16,
    fmt: [u8; 16],
    data: std::ops::Range<usize>,
}

/// Only plain PCM, 8 or 16 bit, mono or stereo. Anything else still plays, but cannot be
/// scaled or repeated.
fn parse_wav(b: &[u8]) -> Option<Wav> {
    if b.len() < 44 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12usize;
    let mut fmt: Option<([u8; 16], u16, u32, u16)> = None;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u32::from_le_bytes(b[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = pos + 8;
        let end = body.checked_add(size)?.min(b.len());
        if id == b"fmt " && end - body >= 16 {
            let f: [u8; 16] = b[body..body + 16].try_into().ok()?;
            let tag = u16::from_le_bytes([f[0], f[1]]);
            let ch = u16::from_le_bytes([f[2], f[3]]);
            let rate = u32::from_le_bytes([f[4], f[5], f[6], f[7]]);
            let bits = u16::from_le_bytes([f[14], f[15]]);
            if tag != 1 || !(1..=2).contains(&ch) || rate == 0 || !(bits == 8 || bits == 16) {
                return None;
            }
            fmt = Some((f, ch, rate, bits));
        } else if id == b"data" {
            let (f, channels, rate, bits) = fmt?;
            return Some(Wav { channels, rate, bits, fmt: f, data: body..end });
        }
        pos = body.checked_add(size)?.checked_add(size & 1)?;
    }
    None
}

/// Length of a (parseable) WAV in milliseconds.
pub fn duration_ms(b: &[u8]) -> Option<u64> {
    let w = parse_wav(b)?;
    let block = (w.bits as u64 / 8) * w.channels as u64;
    Some(w.data.len() as u64 * 1000 / (w.rate as u64 * block))
}

/// Scale the loudness of a WAV (`gain` 0..1). Unsupported formats come back unchanged.
pub fn scale_wav(b: &[u8], gain: f32) -> Vec<u8> {
    let mut out = b.to_vec();
    if gain >= 0.999 {
        return out;
    }
    let Some(w) = parse_wav(b) else { return out };
    let data = &mut out[w.data.clone()];
    if w.bits == 16 {
        for c in data.chunks_exact_mut(2) {
            let v = (i16::from_le_bytes([c[0], c[1]]) as f32 * gain) as i16;
            c.copy_from_slice(&v.to_le_bytes());
        }
    } else {
        for s in data.iter_mut() {
            *s = (((*s as f32 - 128.0) * gain) + 128.0).clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// `times` copies of the clip with `gap_ms` of silence between them.
pub fn repeat_wav(b: &[u8], times: u32, gap_ms: u32) -> Option<Vec<u8>> {
    let w = parse_wav(b)?;
    let block = (w.bits as usize / 8) * w.channels as usize;
    let clip = &b[w.data.clone()];
    let clip = &clip[..clip.len() / block * block];
    let gap = (w.rate as u64 * gap_ms as u64 / 1000) as usize * block;
    let times = times.max(1) as usize;
    let total = (clip.len() + gap) * times;
    if total > MAX_WAV {
        return None;
    }
    let silence = if w.bits == 8 { 0x80 } else { 0u8 };
    let mut data = Vec::with_capacity(total);
    for i in 0..times {
        data.extend_from_slice(clip);
        if i + 1 < times {
            data.extend(std::iter::repeat(silence).take(gap));
        }
    }
    let mut out = wav_header(&w.fmt, data.len());
    out.extend_from_slice(&data);
    Some(out)
}

// ------------------------------------------------------------------------------------- the bank

/// Every sound, ready to play, plus the long (repeated) version of the alarm.
pub struct Bank {
    clips: Vec<Option<Vec<u8>>>,
    alarm_long: Option<Vec<u8>>,
}

impl Bank {
    fn empty() -> Bank {
        Bank { clips: vec![None; Sound::ALL.len()], alarm_long: None }
    }
}

/// Build all sounds for this config. `custom` returns the user's override file for a sound, if any.
pub fn build(cfg: &Config, custom: &dyn Fn(Sound) -> Option<Vec<u8>>) -> Bank {
    let vol = cfg.sound_volume.min(100);
    let mut bank = Bank::empty();
    for s in Sound::ALL {
        let user = custom(s).or_else(|| if s == Sound::Reminder { custom(Sound::Alarm) } else { None });
        let bytes = match user {
            Some(raw) => scale_wav(&raw, vol as f32 / 100.0),
            None => wav_from_samples(&synth(recipe(s, cfg.alarm_tone), cfg.sound_pack, vol)),
        };
        bank.clips[s.idx()] = Some(bytes);
    }
    bank.alarm_long = bank.clips[Sound::Alarm.idx()].as_ref().and_then(|one| match cfg.alarm_repeat {
        AlarmRepeat::Once => None,
        AlarmRepeat::Three => repeat_wav(one, 3, REPEAT_GAP_MS),
        AlarmRepeat::Loop => {
            let dur = duration_ms(one)? as u32 + REPEAT_GAP_MS;
            repeat_wav(one, (LOOP_MS / dur.max(1)).clamp(2, 60), REPEAT_GAP_MS)
        }
    });
    bank
}

fn read_user_file(s: Sound) -> Option<Vec<u8>> {
    let path = crate::config::sounds_dir()?.join(format!("{}.wav", s.stem()));
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_USER_FILE {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    // Must at least look like a WAV; otherwise ignore it and use the generated sound.
    (bytes.len() > 12 && &bytes[0..4] == b"RIFF").then_some(bytes)
}

/// Creates the sounds folder (with a short README) and returns its path.
pub fn prepare_folder() -> Option<PathBuf> {
    let dir = crate::config::sounds_dir()?;
    std::fs::create_dir_all(&dir).ok()?;
    let readme = dir.join("README.txt");
    if !readme.exists() {
        let mut text = String::from(
            "Put your own sounds here. A file with one of these names replaces the built-in sound:\r\n\r\n",
        );
        for s in Sound::ALL {
            text.push_str(&format!("  {}.wav\r\n", s.stem()));
        }
        text.push_str(
            "\r\n\
             - Format: WAV, 8 or 16 bit PCM, mono or stereo, up to 4 MB. Keep them short.\r\n\
             - alarm.wav is repeated automatically (Settings > Sounds > Alarm repeat).\r\n\
             - reminder.wav is optional; without it a reminder plays alarm.wav once.\r\n\
             - The volume setting also applies to your files.\r\n\
             - Delete a file to go back to the built-in sound.\r\n\
             - After adding or removing files: tray menu > Reload config.\r\n",
        );
        let _ = std::fs::write(readme, text);
    }
    Some(dir)
}

// ------------------------------------------------------------------------------------- playback

pub struct Sounds {
    bank: Bank,
    enabled: bool,
    ui: bool,
    events: u8,
    alarm_on: bool,
    /// While an alarm rings, small sounds stay quiet so they do not cut it off.
    busy_until: Option<Instant>,
}

impl Sounds {
    pub fn new(cfg: &Config) -> Sounds {
        let mut s = Sounds { bank: Bank::empty(), enabled: false, ui: false, events: 0, alarm_on: false, busy_until: None };
        s.rebuild(cfg);
        s
    }

    /// Re-read the config and the sounds folder. Costs a few milliseconds, nothing while idle.
    pub fn rebuild(&mut self, cfg: &Config) {
        self.stop();
        raw_stop(); // the old buffers are about to be freed
        self.enabled = cfg.sounds;
        self.ui = cfg.ui_sounds;
        self.events = cfg.sound_events;
        self.alarm_on = cfg.alarm_tone != AlarmTone::Off;
        // Nothing is built while sounds are off, so a silent island holds no audio memory.
        self.bank = if cfg.sounds { build(cfg, &read_user_file) } else { Bank::empty() };
    }

    /// Tick / Open / Close: only with "UI sounds" on.
    pub fn ui(&mut self, s: Sound, now: Instant) {
        if self.ui {
            self.play(s, now, false);
        }
    }

    /// Event sounds, gated by their bit in `sound_events` (`config::sev`).
    pub fn event(&mut self, bit: u8, s: Sound, now: Instant) {
        if self.events & bit != 0 {
            self.play(s, now, false);
        }
    }

    /// Settings feedback: plays regardless of the per-category switches (only the master counts).
    pub fn preview(&mut self, s: Sound, now: Instant) {
        self.play(s, now, true);
    }

    /// Timer finished: the alarm with its repeat setting.
    pub fn alarm(&mut self, now: Instant) {
        if !self.enabled || !self.alarm_on {
            return;
        }
        // The repeated version (3x / loop) if there is one, else the single alarm.
        if let Some(b) = self.bank.alarm_long.as_ref().or(self.bank.clips[Sound::Alarm.idx()].as_ref()) {
            raw_play(b);
            self.busy_until = Some(now + Duration::from_millis(duration_ms(b).unwrap_or(0)));
        }
    }

    /// A reminder came due: one play of the alarm tone.
    pub fn reminder(&mut self, now: Instant) {
        if self.enabled && self.alarm_on {
            self.play(Sound::Reminder, now, true);
        }
    }

    /// Settings preview of the chosen alarm tone: a single play.
    pub fn preview_alarm(&mut self, now: Instant) {
        if self.alarm_on {
            self.play(Sound::Alarm, now, true);
        }
    }

    /// Silence a ringing alarm (called when you click or open the island).
    pub fn stop(&mut self) {
        if self.busy_until.take().is_some() {
            raw_stop();
        }
    }

    fn play(&mut self, s: Sound, now: Instant, force: bool) {
        if !self.enabled {
            return;
        }
        if !force && self.busy_until.is_some_and(|t| now < t) {
            return;
        }
        if let Some(b) = &self.bank.clips[s.idx()] {
            raw_play(b);
            self.busy_until = match s {
                Sound::Alarm | Sound::Reminder => Some(now + Duration::from_millis(duration_ms(b).unwrap_or(0))),
                _ => None,
            };
        }
    }
}

impl Drop for Sounds {
    fn drop(&mut self) {
        raw_stop();
    }
}

#[cfg(windows)]
fn raw_play(wav: &[u8]) {
    use crate::ffi::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    // SND_MEMORY + SND_ASYNC: the buffer must outlive playback, and it does: the bank is only
    // replaced after `raw_stop()` (see `rebuild`).
    unsafe {
        PlaySoundW(wav.as_ptr().cast(), 0, SND_ASYNC | SND_MEMORY | SND_NODEFAULT);
    }
}

#[cfg(windows)]
fn raw_stop() {
    unsafe {
        crate::ffi::PlaySoundW(std::ptr::null(), 0, 0);
    }
}

#[cfg(not(windows))]
fn raw_play(_wav: &[u8]) {}

#[cfg(not(windows))]
fn raw_stop() {}

/// Did any setting that changes the sounds themselves (or when they play) change?
pub fn config_differs(a: &Config, b: &Config) -> bool {
    a.sounds != b.sounds
        || a.sound_volume != b.sound_volume
        || a.sound_pack != b.sound_pack
        || a.ui_sounds != b.ui_sounds
        || a.sound_events != b.sound_events
        || a.alarm_tone != b.alarm_tone
        || a.alarm_repeat != b.alarm_repeat
}

/// Is this one of the sounds that belongs to the "event sounds" chips? (bit, sound), chip order.
pub const EVENT_SOUNDS: [(u8, Sound); 4] = [
    (sev::NOTIFY, Sound::Notify),
    (sev::POWER, Sound::Power),
    (sev::COPY, Sound::Copy),
    (sev::BLUETOOTH, Sound::Bluetooth),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: Sound) -> Option<Vec<u8>> {
        None
    }

    #[test]
    fn stems_are_unique_and_ordered_like_the_enum() {
        for (i, s) in Sound::ALL.iter().enumerate() {
            assert_eq!(s.idx(), i);
            for t in Sound::ALL.iter().skip(i + 1) {
                assert_ne!(s.stem(), t.stem());
            }
        }
    }

    #[test]
    fn recipes_are_sane() {
        for s in Sound::ALL {
            for tone in [AlarmTone::Chime, AlarmTone::Beep, AlarmTone::Bell] {
                let r = recipe(s, tone);
                assert!(!r.is_empty());
                for note in r {
                    assert!(note.hz >= 100.0 && note.hz < RATE as f32 / 2.0, "{s:?} {}", note.hz);
                    assert!(note.at_ms + note.len_ms <= 2000);
                    assert!(note.gain > 0.0 && note.gain <= 1.0);
                }
            }
        }
    }

    #[test]
    fn synth_is_audible_bounded_and_scales_with_volume() {
        for pack in [SoundPack::Soft, SoundPack::Glass, SoundPack::Retro] {
            for tone in [AlarmTone::Chime, AlarmTone::Beep, AlarmTone::Bell] {
                let loud = synth(recipe(Sound::Alarm, tone), pack, 100);
                let soft = synth(recipe(Sound::Alarm, tone), pack, 30);
                let peak = |v: &[i16]| v.iter().map(|x| (*x as i32).abs()).max().unwrap();
                assert!(peak(&loud) > 3000, "{pack:?} {tone:?}");
                assert!(peak(&loud) < 32767);
                assert!(peak(&soft) < peak(&loud));
            }
        }
        assert!(synth(TICK, SoundPack::Soft, 0).iter().all(|&x| x == 0));
        // no click at the very end
        let t = synth(TICK, SoundPack::Retro, 100);
        assert!(t[t.len() - 1].abs() < 100);
    }

    #[test]
    fn generated_wav_parses_back() {
        let samples = synth(NOTIFY, SoundPack::Soft, 60);
        let wav = wav_from_samples(&samples);
        let w = parse_wav(&wav).expect("valid wav");
        assert_eq!((w.channels, w.rate, w.bits), (1, RATE, 16));
        assert_eq!(w.data.len(), samples.len() * 2);
        let ms = duration_ms(&wav).unwrap();
        assert!((350..=420).contains(&ms), "{ms}");
    }

    #[test]
    fn rejects_garbage_and_unsupported_formats() {
        assert!(parse_wav(b"not a wav at all, definitely not a wav file at all....").is_none());
        let mut wav = wav_from_samples(&[0i16; 100]);
        wav[20] = 3; // float format
        assert!(parse_wav(&wav).is_none());
        assert_eq!(scale_wav(&wav, 0.5), wav); // unchanged, still playable
    }

    #[test]
    fn scale_halves_samples() {
        let wav = wav_from_samples(&[1000, -2000, 30000]);
        let half = scale_wav(&wav, 0.5);
        let w = parse_wav(&half).unwrap();
        let v: Vec<i16> = half[w.data].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(v, vec![500, -1000, 15000]);
        assert_eq!(scale_wav(&wav, 1.0), wav);
    }

    #[test]
    fn repeat_adds_gaps_between_copies_only() {
        let one = wav_from_samples(&vec![100i16; RATE as usize / 10]); // 100 ms
        let three = repeat_wav(&one, 3, 200).unwrap();
        let ms = duration_ms(&three).unwrap();
        assert!((ms as i64 - 700).abs() <= 2, "{ms}"); // 3 x 100 + 2 x 200
        assert!(parse_wav(&three).is_some());
        assert_eq!(duration_ms(&repeat_wav(&one, 1, 200).unwrap()), duration_ms(&one));
    }

    #[test]
    fn repeat_refuses_absurd_sizes() {
        let one = wav_from_samples(&vec![1i16; RATE as usize * 5]);
        assert!(repeat_wav(&one, 60, 350).is_none());
    }

    #[test]
    fn build_makes_every_sound_and_alarm_repeats() {
        let mut cfg = Config::default();
        for rep in [AlarmRepeat::Once, AlarmRepeat::Three, AlarmRepeat::Loop] {
            cfg.alarm_repeat = rep;
            let bank = build(&cfg, &none);
            assert!(bank.clips.iter().all(|c| c.is_some()));
            let one = duration_ms(bank.clips[Sound::Alarm.idx()].as_ref().unwrap()).unwrap();
            match rep {
                AlarmRepeat::Once => assert!(bank.alarm_long.is_none()),
                AlarmRepeat::Three => {
                    let l = duration_ms(bank.alarm_long.as_ref().unwrap()).unwrap();
                    assert!(l > one * 2);
                }
                AlarmRepeat::Loop => {
                    let l = duration_ms(bank.alarm_long.as_ref().unwrap()).unwrap();
                    assert!((20_000..=32_000).contains(&l), "{l}");
                }
            }
        }
    }

    #[test]
    fn user_files_override_and_reminder_falls_back_to_alarm() {
        let mine = wav_from_samples(&vec![5000i16; RATE as usize / 2]); // 500 ms
        let only_alarm = |s: Sound| (s == Sound::Alarm).then(|| mine.clone());
        let cfg = Config { sound_volume: 100, alarm_repeat: AlarmRepeat::Once, ..Config::default() };
        let bank = build(&cfg, &only_alarm);
        assert_eq!(bank.clips[Sound::Alarm.idx()].as_ref().unwrap(), &mine);
        assert_eq!(bank.clips[Sound::Reminder.idx()].as_ref().unwrap(), &mine);
        // other sounds stay generated
        assert_ne!(bank.clips[Sound::Tick.idx()].as_ref().unwrap(), &mine);
    }

    #[test]
    fn user_files_follow_the_volume_setting() {
        let mine = wav_from_samples(&[10000, -10000]);
        let all = |_: Sound| Some(mine.clone());
        let cfg = Config { sound_volume: 50, ..Config::default() };
        let bank = build(&cfg, &all);
        let b = bank.clips[Sound::Tick.idx()].as_ref().unwrap();
        let w = parse_wav(b).unwrap();
        assert_eq!(i16::from_le_bytes([b[w.data.start], b[w.data.start + 1]]), 5000);
    }

    #[test]
    fn gating_follows_the_switches() {
        let now = Instant::now();
        let mut cfg = Config::default();
        cfg.sounds = false;
        let mut s = Sounds::new(&cfg);
        s.alarm(now);
        assert!(s.busy_until.is_none()); // master switch off: nothing plays
        cfg.sounds = true;
        cfg.ui_sounds = false;
        s.rebuild(&cfg);
        s.alarm(now);
        assert!(s.busy_until.is_some()); // alarm is independent of UI sounds
        s.stop();
        assert!(s.busy_until.is_none());
        cfg.alarm_tone = AlarmTone::Off;
        s.rebuild(&cfg);
        s.alarm(now);
        assert!(s.busy_until.is_none());
        s.reminder(now);
        assert!(s.busy_until.is_none());
    }

    #[test]
    fn small_sounds_do_not_cut_a_ringing_alarm() {
        let now = Instant::now();
        let cfg = Config::default();
        let mut s = Sounds::new(&cfg);
        s.alarm(now);
        let until = s.busy_until.unwrap();
        s.ui(Sound::Tick, now + Duration::from_millis(10));
        assert_eq!(s.busy_until, Some(until));
        s.rebuild(&cfg);
        assert!(s.busy_until.is_none());
    }

    #[test]
    fn config_differs_sees_every_sound_field() {
        let base = Config::default();
        assert!(!config_differs(&base, &base.clone()));
        let variants = [
            Config { sounds: !base.sounds, ..base.clone() },
            Config { sound_volume: 10, ..base.clone() },
            Config { sound_pack: SoundPack::Retro, ..base.clone() },
            Config { ui_sounds: !base.ui_sounds, ..base.clone() },
            Config { sound_events: 0, ..base.clone() },
            Config { alarm_tone: AlarmTone::Bell, ..base.clone() },
            Config { alarm_repeat: AlarmRepeat::Loop, ..base.clone() },
        ];
        for v in variants {
            assert!(config_differs(&base, &v));
        }
        // unrelated settings do not trigger a rebuild
        assert!(!config_differs(&base, &Config { hover_delay_ms: 5, ..base.clone() }));
    }

    #[test]
    fn event_chips_match_config_bits() {
        let all = EVENT_SOUNDS.iter().fold(0, |a, (b, _)| a | b);
        assert_eq!(all, sev::ALL);
    }
}
