//! Wake Rune when either the daemon or a local I/O job has results to apply.
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake};

use futures::Stream;
use futures::channel::mpsc;
use rune::{Tx, Waker};

use super::{BridgeHandle, PrimeDesktopView};

type Completion = Box<dyn FnOnce(&mut PrimeDesktopView) + Send>;

#[derive(Clone)]
pub(super) struct Poster(mpsc::UnboundedSender<Completion>);

impl Poster {
    pub(super) fn post(&self, complete: impl FnOnce(&mut PrimeDesktopView) + Send + 'static) {
        let _ = self.0.unbounded_send(Box::new(complete));
    }
}

pub(super) struct Background {
    tx: mpsc::UnboundedSender<Completion>,
    rx: mpsc::UnboundedReceiver<Completion>,
}

impl Background {
    pub(super) fn new() -> Self {
        let (tx, rx) = mpsc::unbounded();
        Self { tx, rx }
    }

    pub(super) fn poster(&self) -> Poster {
        Poster(self.tx.clone())
    }

    pub(super) fn run<F, Done>(&self, bridge: &BridgeHandle, work: F)
    where
        F: FnOnce() -> Done + Send + 'static,
        Done: FnOnce(&mut PrimeDesktopView) + Send + 'static,
    {
        let tx = self.tx.clone();
        bridge.spawn_blocking(move || {
            let _ = tx.unbounded_send(Box::new(work()));
        });
    }
}

struct WindowWake(Waker);

impl Wake for WindowWake {
    fn wake(self: Arc<Self>) {
        (self.0)();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        (self.0)();
    }
}

impl PrimeDesktopView {
    pub fn set_waker(&mut self, wake: Waker) {
        self.waker = Arc::new(WindowWake(wake)).into();
        self.waker.wake_by_ref();
    }

    /// The stream registers the window's waker whenever it runs dry. A hot stream
    /// yields after a bounded batch so input and layout can still make progress.
    pub fn drain_events(&mut self) -> bool {
        let waker = self.waker.clone();
        let mut cx = Context::from_waker(&waker);
        let mut changed = false;
        for _ in 0..128 {
            let event = Pin::new(&mut self.events).poll_next(&mut cx);
            let completion = Pin::new(&mut self.background.rx).poll_next(&mut cx);
            if !matches!(event, Poll::Ready(Some(_))) && !matches!(completion, Poll::Ready(Some(_)))
            {
                return changed;
            }
            if let Poll::Ready(Some(event)) = event {
                self.apply_bridge_event(event);
            }
            if let Poll::Ready(Some(complete)) = completion {
                complete(self);
            }
            changed = true;
        }
        self.waker.wake_by_ref();
        changed
    }

    pub fn poll(&mut self, _time: f64, tx: &mut Tx) -> bool {
        let mut changed = self.drain_events();
        if std::mem::take(&mut self.scroll_to_tail) {
            tx.scroll_to_bottom_instant(self.transcript_viewport_key());
            changed = true;
        }
        if let Some(dialog) = self.pending_dialog.take() {
            self.show_dialog(dialog, tx);
            return true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::workspace::LaunchOptions;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn sleeping_window_wakes_for_completed_io_without_polling_a_timer() {
        let profile = tempfile::tempdir().unwrap();
        let mut view = PrimeDesktopView::new(
            LaunchOptions {
                daemon_socket: Some(profile.path().join("offline.sock")),
                initial_cwd: Some(profile.path().into()),
                ..Default::default()
            },
            profile.path().into(),
        );
        let (wake, wakes) = mpsc::channel();
        view.set_waker(Arc::new(move || {
            let _ = wake.send(());
        }));
        let (release, start) = mpsc::channel();
        view.background.run(&view.bridge, move || {
            start
                .recv_timeout(Duration::from_secs(5))
                .expect("release I/O");
            |s: &mut PrimeDesktopView| s.model_picker_configured_only = true
        });
        // Register the shell waker on both event streams before the worker completes.
        view.drain_events();
        while wakes.try_recv().is_ok() {}
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !view.model_picker_configured_only {
            wakes
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("shell wake");
            view.drain_events();
        }
    }
}
