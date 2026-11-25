//! Now-playing data and transport control through Windows' Global System Media Transport
//! Controls (GSMTC): the same API that drives the volume flyout and lock-screen media widget.
//!
//! A worker thread subscribes to the change events and sleeps on a channel otherwise, so it costs
//! nothing while nothing happens. Results go into a mutex-protected snapshot and the UI thread is
//! woken with a posted message.

use crate::ffi::{CoInitializeEx, PostMessageW, HANDLE};
use crate::gfx::resample_box;
use crate::util::{accent_from_avg, avg_color_premul, crop_square_bgra_to_rgba, friendly_app};
use crate::win::{filetime_now, WM_APP_MEDIA};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tiny_skia::Pixmap;
use windows::core::Interface;
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform, ColorManagementMode,
    ExifOrientationMode,
};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionMediaProperties as MediaProps,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PbStatus,
};
use windows::Storage::Streams::IRandomAccessStream;

/// Edge length of the album-art texture kept in RAM (64 KB).
const ART_SIDE: u32 = 128;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MediaInfo {
    pub has_session: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Friendly source app name ("Spotify", "Chrome", ...).
    pub app: String,
    pub playing: bool,
    pub can_prev: bool,
    pub can_next: bool,
    pub can_toggle: bool,
    pub can_seek: bool,
    pub pos_ms: i64,
    pub dur_ms: i64,
    /// Bumps whenever the track (title/artist/album/app) changes.
    pub track_id: u64,
}

pub struct Art {
    /// Premultiplied RGBA, `side` x `side`.
    pub rgba: Vec<u8>,
    pub side: u32,
    pub accent: (u8, u8, u8),
}

#[derive(Default)]
pub struct Shared {
    pub info: MediaInfo,
    /// `None` = unchanged, `Some(None)` = clear the art, `Some(Some(a))` = new art.
    pub art_update: Option<Option<Art>>,
}

pub enum Cmd {
    TogglePlay,
    Next,
    Prev,
    /// Absolute position in milliseconds.
    Seek(i64),
}

pub enum Msg {
    Refresh,
    Cmd(Cmd),
    /// Turn album-art loading on or off at run time (Settings tab).
    SetArt(bool),
    Quit,
}

/// UI-thread copy of the media state.
pub struct MediaView {
    pub info: MediaInfo,
    /// When `info.pos_ms` was captured, for extrapolating the progress bar.
    pub at: Instant,
    pub art: Option<Pixmap>,
    pub accent: Option<(u8, u8, u8)>,
}

impl MediaView {
    pub fn new() -> Self {
        Self { info: MediaInfo::default(), at: Instant::now(), art: None, accent: None }
    }

    pub fn pos_ms(&self, now: Instant) -> i64 {
        let mut p = self.info.pos_ms;
        if self.info.playing {
            p += now.saturating_duration_since(self.at).as_millis() as i64;
        }
        if self.info.dur_ms > 0 {
            p = p.min(self.info.dur_ms);
        }
        p.max(0)
    }
}

pub fn spawn(hwnd: HANDLE, want_art: bool) -> (Sender<Msg>, Arc<Mutex<Shared>>) {
    let (tx, rx) = channel();
    let shared = Arc::new(Mutex::new(Shared::default()));
    let (tx2, sh2) = (tx.clone(), shared.clone());
    let _ = std::thread::Builder::new()
        .name("media".into())
        .stack_size(1 << 20)
        .spawn(move || run(hwnd, tx2, rx, sh2, want_art));
    (tx, shared)
}

struct Bound {
    session: Session,
    t_props: EventRegistrationToken,
    t_play: EventRegistrationToken,
    t_time: EventRegistrationToken,
}

struct Worker {
    manager: Manager,
    tx: Sender<Msg>,
    hwnd: HANDLE,
    shared: Arc<Mutex<Shared>>,
    want_art: bool,
    bound: Option<Bound>,
    last_key: String,
    track_id: u64,
    have_art: bool,
    art_tries: u32,
    /// Fingerprint + album of the cover shown for the previous track: a player that publishes
    /// the new title before the new thumbnail still hands out the old cover for a moment.
    prev_art: Option<(u64, String)>,
    cur_art: Option<(u64, String)>,
    last_art_try: Option<Instant>,
    playing: bool,
}

/// Cover retries for a new track (one every `ART_RETRY`), before giving up / accepting.
const ART_TRIES: u32 = 6;
const ART_RETRY: Duration = Duration::from_millis(400);

fn art_hash(a: &Art) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    a.side.hash(&mut h);
    a.rgba.hash(&mut h);
    h.finish()
}

/// Is a freshly loaded cover just the previous track's cover still being served? Same album
/// legitimately shares the cover; the last try accepts whatever the player offers.
pub fn stale_cover(prev: Option<&(u64, String)>, hash: u64, album: &str, last_try: bool) -> bool {
    match prev {
        Some((h, prev_album)) => *h == hash && !last_try && (album.is_empty() || album != prev_album),
        None => false,
    }
}

fn run(hwnd: HANDLE, tx: Sender<Msg>, rx: Receiver<Msg>, shared: Arc<Mutex<Shared>>, want_art: bool) {
    unsafe { CoInitializeEx(std::ptr::null_mut(), 0) }; // COINIT_MULTITHREADED
    let manager = match Manager::RequestAsync().and_then(|op| op.get()) {
        Ok(m) => m,
        Err(_) => return,
    };
    {
        let tx = tx.clone();
        let _ = manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
            let _ = tx.send(Msg::Refresh);
            Ok(())
        }));
    }
    let mut w = Worker {
        manager,
        tx,
        hwnd,
        shared,
        want_art,
        bound: None,
        last_key: String::new(),
        track_id: 0,
        have_art: false,
        art_tries: 0,
        prev_art: None,
        cur_art: None,
        last_art_try: None,
        playing: false,
    };
    w.refresh();
    loop {
        // Events wake us instantly; the timeout only re-syncs the position of a playing track.
        let wait = if w.art_pending() {
            ART_RETRY
        } else if w.playing {
            Duration::from_secs(10)
        } else {
            Duration::from_secs(60)
        };
        match rx.recv_timeout(wait) {
            Ok(Msg::Quit) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Msg::Cmd(c)) => {
                w.command(c);
                std::thread::sleep(Duration::from_millis(120)); // let the player apply it
            }
            Ok(Msg::SetArt(v)) => w.set_art(v),
            Ok(Msg::Refresh) => std::thread::sleep(Duration::from_millis(40)), // coalesce event bursts
            Err(RecvTimeoutError::Timeout) => {}
        }
        let mut quit = false;
        while let Ok(m) = rx.try_recv() {
            match m {
                Msg::Cmd(c) => w.command(c),
                Msg::SetArt(v) => w.set_art(v),
                Msg::Quit => quit = true,
                Msg::Refresh => {}
            }
        }
        if quit {
            break;
        }
        w.refresh();
    }
}

impl Worker {
    fn art_pending(&self) -> bool {
        self.want_art && !self.have_art && !self.last_key.is_empty() && self.art_tries < ART_TRIES
    }

    fn set_art(&mut self, on: bool) {
        self.want_art = on;
        if on {
            // Fetch the cover of the current track on the next refresh.
            self.have_art = false;
            self.art_tries = 0;
            self.last_art_try = None;
        }
    }

    fn command(&mut self, c: Cmd) {
        let Ok(s) = self.manager.GetCurrentSession() else {
            return;
        };
        match c {
            Cmd::TogglePlay => {
                let _ = s.TryTogglePlayPauseAsync();
            }
            Cmd::Next => {
                let _ = s.TrySkipNextAsync();
            }
            Cmd::Prev => {
                let _ = s.TrySkipPreviousAsync();
            }
            Cmd::Seek(ms) => {
                let start = s
                    .GetTimelineProperties()
                    .and_then(|t| t.StartTime())
                    .map(|t| t.Duration)
                    .unwrap_or(0);
                let _ = s.TryChangePlaybackPositionAsync(start + ms.max(0) * 10_000);
            }
        }
    }

    /// Follow the current session: (re)subscribe to its change events when it changes.
    fn rebind(&mut self, session: Option<&Session>) {
        let same = match (&self.bound, session) {
            (Some(b), Some(s)) => &b.session == s,
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        if let Some(b) = self.bound.take() {
            let _ = b.session.RemoveMediaPropertiesChanged(b.t_props);
            let _ = b.session.RemovePlaybackInfoChanged(b.t_play);
            let _ = b.session.RemoveTimelinePropertiesChanged(b.t_time);
        }
        if let Some(s) = session {
            let tx = self.tx.clone();
            let t_props = s.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx.send(Msg::Refresh);
                Ok(())
            }));
            let tx = self.tx.clone();
            let t_play = s.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx.send(Msg::Refresh);
                Ok(())
            }));
            let tx = self.tx.clone();
            let t_time = s.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                let _ = tx.send(Msg::Refresh);
                Ok(())
            }));
            self.bound = Some(Bound {
                session: s.clone(),
                t_props: t_props.unwrap_or(EventRegistrationToken { Value: 0 }),
                t_play: t_play.unwrap_or(EventRegistrationToken { Value: 0 }),
                t_time: t_time.unwrap_or(EventRegistrationToken { Value: 0 }),
            });
            self.last_key.clear();
            self.have_art = false;
            self.art_tries = 0;
        }
    }

    fn refresh(&mut self) {
        let session = self.manager.GetCurrentSession().ok();
        self.rebind(session.as_ref());
        let mut info = MediaInfo::default();
        let mut art_update: Option<Option<Art>> = None;
        if let Some(s) = &session {
            let _ = self.read(s, &mut info, &mut art_update);
        } else if !self.last_key.is_empty() {
            // The session went away: forget the track and drop its art.
            self.last_key.clear();
            self.have_art = false;
            if self.want_art {
                art_update = Some(None);
            }
        }
        info.track_id = self.track_id;
        self.playing = info.playing;
        {
            let mut g = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            g.info = info;
            if art_update.is_some() {
                g.art_update = art_update;
            }
        }
        unsafe { PostMessageW(self.hwnd, WM_APP_MEDIA, 0, 0) };
    }

    fn read(&mut self, s: &Session, info: &mut MediaInfo, art_update: &mut Option<Option<Art>>) -> windows::core::Result<()> {
        let pb = s.GetPlaybackInfo()?;
        let status = pb.PlaybackStatus()?;
        if status == PbStatus::Closed {
            // GSMTC can keep "the current session" pointed at an app that has actually closed,
            // still answering with that app's *last* track - show nothing rather than stale info.
            if !self.last_key.is_empty() {
                self.last_key.clear();
                self.have_art = false;
                if self.want_art {
                    *art_update = Some(None);
                }
            }
            return Ok(()); // `info` stays MediaInfo::default(): has_session == false
        }

        info.has_session = true;
        info.app = friendly_app(&s.SourceAppUserModelId()?.to_string());
        info.playing = status == PbStatus::Playing;
        if let Ok(c) = pb.Controls() {
            info.can_prev = c.IsPreviousEnabled().unwrap_or(false);
            info.can_next = c.IsNextEnabled().unwrap_or(false);
            info.can_toggle = c.IsPlayPauseToggleEnabled().unwrap_or(false)
                || c.IsPlayEnabled().unwrap_or(false)
                || c.IsPauseEnabled().unwrap_or(false);
            info.can_seek = c.IsPlaybackPositionEnabled().unwrap_or(false);
        }

        // A failed read counts as "no metadata": never keep the previous track's text or cover.
        let props = s.TryGetMediaPropertiesAsync().and_then(|op| op.get()).ok();
        if let Some(p) = &props {
            info.title = p.Title().map(|h| h.to_string()).unwrap_or_default();
            info.artist = p.Artist().map(|h| h.to_string()).unwrap_or_default();
            info.album = p.AlbumTitle().map(|h| h.to_string()).unwrap_or_default();
        }
        let key = format!("{}\u{1}{}\u{1}{}\u{1}{}", info.app, info.title, info.artist, info.album);
        if key != self.last_key {
            self.last_key = key;
            self.track_id += 1;
            self.have_art = false;
            self.art_tries = 0;
            self.last_art_try = None;
            if self.cur_art.is_some() {
                self.prev_art = self.cur_art.take();
            }
            if self.want_art {
                *art_update = Some(None); // never show the previous track's cover
            }
        }
        // Players often publish the thumbnail a moment after the text: retry for a few seconds.
        // Every attempt (also a failed metadata read) spends a try, paced by ART_RETRY so bursts of
        // media events cannot use up the budget before the new thumbnail arrives.
        let due = self.last_art_try.map_or(true, |t| t.elapsed() >= ART_RETRY);
        if self.want_art && !self.have_art && self.art_tries < ART_TRIES && due {
            self.art_tries += 1;
            self.last_art_try = Some(Instant::now());
            if let Some(a) = props.as_ref().and_then(load_art) {
                let h = art_hash(&a);
                if !stale_cover(self.prev_art.as_ref(), h, &info.album, self.art_tries >= ART_TRIES) {
                    self.cur_art = Some((h, info.album.clone()));
                    *art_update = Some(Some(a));
                    self.have_art = true;
                }
            }
        }

        if let Ok(tl) = s.GetTimelineProperties() {
            let start = tl.StartTime().map(|t| t.Duration).unwrap_or(0);
            let end = tl.EndTime().map(|t| t.Duration).unwrap_or(0);
            let pos = tl.Position().map(|t| t.Duration).unwrap_or(0);
            let updated = tl.LastUpdatedTime().map(|t| t.UniversalTime).unwrap_or(0);
            let dur_ms = (end - start).max(0) / 10_000;
            let mut pos_ms = (pos - start).max(0) / 10_000;
            if info.playing && updated > 0 {
                // Position is as of LastUpdatedTime; bring it up to now.
                pos_ms += (filetime_now() - updated).max(0) / 10_000;
            }
            if dur_ms > 0 {
                pos_ms = pos_ms.min(dur_ms);
            }
            info.pos_ms = pos_ms;
            info.dur_ms = dur_ms;
        }
        if info.dur_ms <= 0 {
            info.can_seek = false;
        }
        Ok(())
    }
}

/// Decode the session thumbnail with the Windows imaging stack (JPEG/PNG/WebP/...), scale it down
/// in the decoder, centre-crop to a square and shrink to the art texture size.
fn load_art(props: &MediaProps) -> Option<Art> {
    let reference = props.Thumbnail().ok()?;
    let stream = reference.OpenReadAsync().ok()?.get().ok()?;
    let stream: IRandomAccessStream = stream.cast().ok()?;
    let decoder = BitmapDecoder::CreateAsync(&stream).ok()?.get().ok()?;
    let (w, h) = (decoder.PixelWidth().ok()?, decoder.PixelHeight().ok()?);
    if w == 0 || h == 0 {
        return None;
    }
    let short = w.min(h) as u64;
    let (tw, th) =
        if short > 256 { ((w as u64 * 256 / short) as u32, (h as u64 * 256 / short) as u32) } else { (w, h) };
    let tf = BitmapTransform::new().ok()?;
    tf.SetScaledWidth(tw).ok()?;
    tf.SetScaledHeight(th).ok()?;
    tf.SetInterpolationMode(BitmapInterpolationMode::Fant).ok()?;
    let provider = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Premultiplied,
            &tf,
            ExifOrientationMode::IgnoreExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )
        .ok()?
        .get()
        .ok()?;
    let bytes = provider.DetachPixelData().ok()?;
    let raw: &[u8] = &bytes;
    if raw.len() < (tw as usize) * (th as usize) * 4 {
        return None;
    }
    let (square, side) = crop_square_bgra_to_rgba(raw, tw, th);
    if side == 0 {
        return None;
    }
    let rgba = resample_box(&square, side, side, ART_SIDE, ART_SIDE);
    let (r, g, b) = avg_color_premul(&rgba);
    Some(Art { rgba, side: ART_SIDE, accent: accent_from_avg(r, g, b) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_cover_rules() {
        let prev = (42u64, "Album A".to_string());
        assert!(stale_cover(Some(&prev), 42, "Album B", false)); // old cover, new album: wait
        assert!(stale_cover(Some(&prev), 42, "", false)); // unknown album: wait
        assert!(!stale_cover(Some(&prev), 42, "Album A", false)); // same album shares the cover
        assert!(!stale_cover(Some(&prev), 7, "Album B", false)); // a different cover
        assert!(!stale_cover(Some(&prev), 42, "Album B", true)); // last try accepts
        assert!(!stale_cover(None, 42, "", false));
    }
}
