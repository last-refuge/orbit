//! `orbit preview`: live previews of Orbit's fixture windows in a GPUI
//! window (ORB-004). Capture, conversion and downscaling run on a worker
//! thread with its own Wayland connection; the UI thread only swaps
//! ready-made BGRA thumbnails and frees the textures it replaces.

use std::{
    collections::HashMap,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use futures::{StreamExt, channel::mpsc};
use gpui::{
    App, Bounds, Context, FocusHandle, KeyBinding, ObjectFit, RenderImage, SharedString, Window,
    WindowBounds, WindowOptions, actions, div, img, prelude::*, px, size,
};
use orbit_capture::{Capturer, Failure, Session};
use orbit_ui::theme;
use smallvec::SmallVec;

use crate::capture::{PREVIEW_APP_ID, PREVIEW_NAME, Verdict, fixture_name, judge, rss_kib};

/// Thumbnails are bounded so GPU memory stays proportional to card count,
/// not window size.
const THUMB_MAX: (u32, u32) = (640, 400);

/// Repeat frames arrive only after new damage; don't hold the loop for them.
const REPEAT_WAIT: Duration = Duration::from_millis(20);

actions!(orbit_preview, [Close]);

pub struct Options {
    pub interval: Duration,
    pub exit_after: Option<Duration>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(1000),
            exit_after: None,
        }
    }
}

struct Card {
    name: SharedString,
    image: Option<Arc<RenderImage>>,
    thumb: (u32, u32),
    full: (u32, u32),
    capture_ms: f64,
    verdict: Verdict,
    error: Option<String>,
    /// No new damage since the last frame: keep showing the previous image.
    unchanged: bool,
}

struct Batch {
    cards: Vec<Card>,
    elapsed_ms: f64,
    /// Capture as a whole is unavailable (the cards are empty).
    error: Option<String>,
}

pub fn run(options: Options) {
    let (tx, rx) = mpsc::unbounded::<Batch>();
    let interval = options.interval;
    thread::Builder::new()
        .name("orbit-capture".into())
        .spawn(move || {
            if let Err(error) = capture_loop(&tx, interval) {
                eprintln!("orbit preview: {error:#}");
                let _ = tx.unbounded_send(Batch {
                    cards: Vec::new(),
                    elapsed_ms: 0.0,
                    error: Some(format!("{error:#}")),
                });
            }
        })
        .expect("spawning the capture thread");

    gpui_platform::application().run(move |cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Close, None)]);
        cx.on_action(|_: &Close, cx| cx.quit());
        let mut rx = Some(rx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1100.), px(720.)),
                    cx,
                ))),
                app_id: Some(PREVIEW_APP_ID.into()),
                ..Default::default()
            },
            |window, cx| {
                cx.new(|cx| {
                    let view = Preview::new(window, cx);
                    let mut rx = rx.take().expect("one window");
                    cx.spawn_in(window, async move |this, cx| {
                        while let Some(batch) = rx.next().await {
                            if this
                                .update_in(cx, |view, window, cx| view.replace(batch, window, cx))
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                    view
                })
            },
        )
        .expect("failed to open the preview window");

        if let Some(delay) = options.exit_after {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(delay).await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
}

/// Keeps one session per fixture so buffers are reused between refreshes.
fn capture_loop(tx: &mpsc::UnboundedSender<Batch>, interval: Duration) -> anyhow::Result<()> {
    let mut capturer = Capturer::connect()?;
    let mut sessions: HashMap<String, Session> = HashMap::new();
    loop {
        let started = Instant::now();
        let toplevels = capturer.toplevels()?;
        let mut cards = Vec::new();
        let mut alive = Vec::new();
        for toplevel in &toplevels {
            let Some(name) = fixture_name(toplevel).filter(|name| *name != PREVIEW_NAME) else {
                continue;
            };
            alive.push(toplevel.identifier.clone());
            let session = match sessions.remove(&toplevel.identifier) {
                Some(session) => Ok(session),
                None => capturer.open(toplevel),
            };
            let card = match session {
                Ok(mut session) => match capturer.grab(&mut session, REPEAT_WAIT) {
                    Ok(capture) => {
                        sessions.insert(toplevel.identifier.clone(), session);
                        let (verdict, _, _) = judge(name, &capture.frame);
                        let thumb = thumbnail(&capture.frame, THUMB_MAX);
                        Card {
                            name: name.to_owned().into(),
                            thumb: (thumb.width(), thumb.height()),
                            image: Some(Arc::new(RenderImage::new(SmallVec::from_elem(
                                image::Frame::new(thumb),
                                1,
                            )))),
                            full: (capture.width, capture.height),
                            capture_ms: capture.timing.capture_ms,
                            verdict,
                            error: None,
                            unchanged: false,
                        }
                    }
                    Err((Failure::Unchanged, _)) => {
                        sessions.insert(toplevel.identifier.clone(), session);
                        Card {
                            unchanged: true,
                            ..failed_card(name, anyhow::anyhow!("unchanged"))
                        }
                    }
                    Err((_, error)) => {
                        // Drop the session; the next round opens a fresh one.
                        capturer.close(session);
                        failed_card(name, error)
                    }
                },
                Err(error) => failed_card(name, error),
            };
            cards.push(card);
        }
        for identifier in sessions.keys().cloned().collect::<Vec<_>>() {
            if !alive.contains(&identifier) {
                let session = sessions.remove(&identifier).expect("listed");
                capturer.close(session);
            }
        }
        cards.sort_by(|a, b| a.name.cmp(&b.name));
        let batch = Batch {
            cards,
            elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            error: None,
        };
        if tx.unbounded_send(batch).is_err() {
            return Ok(());
        }
        thread::sleep(interval.saturating_sub(started.elapsed()));
    }
}

fn failed_card(name: &str, error: anyhow::Error) -> Card {
    Card {
        name: name.to_owned().into(),
        image: None,
        thumb: (0, 0),
        full: (0, 0),
        capture_ms: 0.0,
        verdict: Verdict::Failed,
        error: Some(format!("{error:#}")),
        unchanged: false,
    }
}

/// Downscales once, area-averaged, to fit inside `max` (device pixels),
/// keeping the frame's aspect; never upscales. Channel order passes through
/// untouched, so the BGRA bytes stay BGRA for GPUI. The result is the
/// size the overview draws it at, so nothing is resampled twice.
pub fn thumbnail(frame: &orbit_capture::Frame, max: (u32, u32)) -> image::RgbaImage {
    let full = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra.clone())
        .expect("frame size matches its pixels");
    let (width, height) = fit(frame.width, frame.height, max);
    if (width, height) == (frame.width, frame.height) {
        return full;
    }
    image::imageops::thumbnail(&full, width, height)
}

/// The largest size with `width:height`'s aspect that fits in `max`
/// without growing. Width leads, so a 16:10 frame in a 16:10 box fills it.
pub fn fit(width: u32, height: u32, max: (u32, u32)) -> (u32, u32) {
    let (max_w, max_h) = (max.0.max(1), max.1.max(1));
    if width == 0 || height == 0 {
        return (1, 1);
    }
    let scale = (f64::from(max_w) / f64::from(width))
        .min(f64::from(max_h) / f64::from(height))
        .min(1.0);
    if scale >= 1.0 {
        return (width, height);
    }
    let w = ((f64::from(width) * scale).round() as u32).clamp(1, max_w);
    let h = ((f64::from(height) * scale).round() as u32).clamp(1, max_h);
    (w, h)
}

struct Preview {
    cards: Vec<Card>,
    refreshes: usize,
    /// Cards whose image changed in the last refresh.
    updated: usize,
    last_ms: f64,
    error: Option<String>,
    focus_handle: FocusHandle,
}

impl Preview {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self {
            cards: Vec::new(),
            refreshes: 0,
            updated: 0,
            last_ms: 0.0,
            error: None,
            focus_handle,
        }
    }

    fn replace(&mut self, batch: Batch, window: &mut Window, cx: &mut Context<Self>) {
        let mut old: HashMap<SharedString, Card> = std::mem::take(&mut self.cards)
            .into_iter()
            .map(|card| (card.name.clone(), card))
            .collect();
        self.updated = 0;
        for card in batch.cards {
            if !card.unchanged {
                self.updated += 1;
            }
            let card = match old.remove(&card.name) {
                Some(previous) if card.unchanged => previous,
                Some(previous) => {
                    // Free the replaced texture now instead of leaving it in the atlas.
                    if let Some(image) = previous.image {
                        cx.drop_image(image, Some(window));
                    }
                    card
                }
                None => card,
            };
            self.cards.push(card);
        }
        for gone in old.into_values() {
            if let Some(image) = gone.image {
                cx.drop_image(image, Some(window));
            }
        }
        self.refreshes += 1;
        self.last_ms = batch.elapsed_ms;
        self.error = batch.error;
        cx.notify();
    }

    fn texture_bytes(&self) -> u64 {
        self.cards
            .iter()
            .map(|c| u64::from(c.thumb.0) * u64::from(c.thumb.1) * 4)
            .sum()
    }
}

impl Render for Preview {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let footer = format!(
            "{} cards ({} updated) · refresh {} took {:.1} ms · textures {:.1} MiB · RSS {} MiB · Escape closes",
            self.cards.len(),
            self.updated,
            self.refreshes,
            self.last_ms,
            self.texture_bytes() as f64 / (1024.0 * 1024.0),
            rss_kib()
                .map(|k| (k / 1024).to_string())
                .unwrap_or("?".into()),
        );
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::MIDNIGHT.rgba())
            .text_color(theme::CLOUD.rgba())
            .p_4()
            .gap_3()
            .child(
                div()
                    .text_size(px(20.))
                    .child("Orbit capture preview (fixtures only)"),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .text_color(theme::CORAL.rgba())
                    .child(format!("Previews unavailable: {error}"))
            }))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_4()
                    .children(self.cards.iter().map(card)),
            )
            .child(
                div()
                    .mt_auto()
                    .text_size(px(12.))
                    .text_color(theme::SILVER.rgba())
                    .child(footer),
            )
    }
}

fn card(card: &Card) -> impl IntoElement {
    let caption = match &card.error {
        Some(error) => format!("{} · {error}", card.name),
        None => format!(
            "{} · {:?} · {}x{} → {}x{} · {:.1} ms",
            card.name,
            card.verdict,
            card.full.0,
            card.full.1,
            card.thumb.0,
            card.thumb.1,
            card.capture_ms
        ),
    };
    let border = if card.verdict == Verdict::Ok {
        theme::SLATE
    } else {
        theme::CORAL
    };
    div()
        .w(px(320.))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .h(px(200.))
                .w_full()
                .rounded_md()
                .border_2()
                .border_color(border.rgba())
                .bg(theme::SLATE.rgba())
                .overflow_hidden()
                .children(
                    card.image
                        .clone()
                        .map(|image| img(image).size_full().object_fit(ObjectFit::Contain)),
                )
                .when(card.image.is_none(), |d| {
                    d.flex()
                        .items_center()
                        .justify_center()
                        .child("Preview unavailable")
                }),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(theme::SILVER.rgba())
                .child(caption),
        )
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn fits_inside_the_box_without_growing() {
        assert_eq!(fit(2880, 1800, (650, 406)), (650, 406));
        assert_eq!(fit(2880, 1800, (650, 500)), (650, 406));
        assert_eq!(fit(1440, 1800, (650, 500)), (400, 500));
        assert_eq!(fit(480, 320, (650, 500)), (480, 320));
        assert_eq!(fit(0, 0, (650, 500)), (1, 1));
        assert_eq!(fit(10, 10, (0, 0)), (1, 1));
    }
}
