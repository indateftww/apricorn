//! The app contract — deterministic scene logic over the frame model.
//!
//! An [`App`] is one scene (Phase 3: the intro's copyright beat, then
//! the title screen): it advances on every tick, produces the logical
//! frame for that tick, and says when it is done. A [`BootChain`]
//! strings apps together in boot order, advancing when each finishes
//! and cycling forever after the last — the title screen's idle
//! timeout returns to the intro, as the game's own chain does.
//!
//! The contract is the seam the future harness drives: ticks are pure
//! functions of the frame index and the input, so replaying a frame
//! range with the same inputs reproduces the same logical frames.

use crate::assets::AssetStore;
use crate::frame::LogicalFrame;
use crate::input::Input;

pub mod intro_copyright;
pub mod title_screen;

/// What a [`BootChain`] does after the current app's tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainNext {
    /// Keep running the current app.
    Stay,
    /// The app has finished — the chain advances to the next.
    Advance,
}

/// One scene: consumes a tick's input, produces the logical frame.
///
/// The two halves of the tick are split so a runner can hold the
/// frame while advancing (a mutable borrow ends at [`App::next`]):
///
/// * [`App::tick`] advances the scene state for one frame;
/// * [`App::frame`] returns the logical frame the *last* tick produced
///   (valid from construction, before any tick, as the scene's cleared
///   state — both Phase 3 apps start from a black screen);
/// * [`App::next`] says whether the chain should advance after this
///   tick.
///
/// Ticks must be deterministic: the same frame index and input
/// sequence produce the same state, no wall-clock, no RNG.
pub trait App {
    /// Advances the scene one tick. `frame` is the global tick token
    /// (its index, the zero-based frame since boot).
    fn tick(&mut self, frame: crate::Frame, input: Input);

    /// The logical frame the last tick produced (or the scene's
    /// cleared initial state before the first).
    fn frame(&self) -> &LogicalFrame;

    /// Whether this app has finished and the chain should advance.
    fn next(&self) -> ChainNext;
}

/// The Phase 3 boot chain: the intro's copyright beat, then the
/// title screen, cycling forever (the title's timeout exits back to
/// the intro, as pret's `TITLESCREEN_EXIT_TIMEOUT` does).
///
/// Both factories share `store` (an `Arc<Mutex<…>>`, since the
/// presenter renders from the store while the chain constructs the
/// next app at an advance) and load their members fresh on every
/// construction, so a cycle is a true reset of both state and
/// handles. A load failure panics: the store only opens the
/// SHA-1-pinned dump the asset tables are valid for, so a broken
/// member is a broken table, not a runtime condition.
#[must_use]
pub fn boot_chain(store: std::sync::Arc<std::sync::Mutex<AssetStore>>) -> BootChain {
    let intro = std::sync::Arc::clone(&store);
    let title = std::sync::Arc::clone(&store);
    BootChain::new(vec![
        Box::new(move || {
            Box::new(
                intro_copyright::IntroCopyright::load(&intro)
                    .expect("the pinned ROM's copyright-beat members load"),
            )
        }),
        Box::new(move || {
            Box::new(
                title_screen::TitleScreen::load(&title)
                    .expect("the pinned ROM's title-screen members load"),
            )
        }),
    ])
}

/// The boot-order runner: intro → title → (on timeout) back to intro.
///
/// Holds one *factory* per app, not an instance: when the chain
/// advances (or cycles), it constructs the next app fresh, so a
/// restart is a true reset with no leftover state — the determinism
/// the harness needs when a replay crosses a chain boundary.
///
/// The tick that advances returns the *finishing* app's last frame;
/// the next tick belongs to the new app. The cycle wraps the last app
/// back to the first (a single-app chain restarts itself).
pub struct BootChain {
    factories: Vec<Box<dyn Fn() -> Box<dyn App>>>,
    current: Box<dyn App>,
    index: usize,
    /// The finishing app's last frame, kept alive across the swap so
    /// an advancing tick still shows what the finishing app drew.
    last: Option<LogicalFrame>,
}

impl BootChain {
    /// Builds a chain from app factories, in boot order; the first
    /// is constructed immediately and ticking starts there.
    ///
    /// # Panics
    /// Panics when `factories` is empty — a chain must start somewhere.
    pub fn new(factories: Vec<Box<dyn Fn() -> Box<dyn App>>>) -> Self {
        let first = factories
            .first()
            .expect("a boot chain needs at least one app")();
        Self {
            factories,
            current: first,
            index: 0,
            last: None,
        }
    }

    /// The app currently running (its logical frame is the chain's).
    #[must_use]
    pub fn app(&self) -> &dyn App {
        &*self.current
    }

    /// Advances the chain one tick, advancing apps when they finish.
    ///
    /// Returns the logical frame this tick produced: the current
    /// app's frame, or on an advancing tick the finishing app's last
    /// frame — the next tick belongs to the new app.
    pub fn tick(&mut self, frame: crate::Frame, input: Input) -> &LogicalFrame {
        self.current.tick(frame, input);
        if self.current.next() != ChainNext::Advance {
            return self.current.frame();
        }
        // Retain the finishing frame (the frame is plain data), then
        // construct the next app fresh — a cycle wraps to the first
        // factory, so the restart is a reset.
        self.last = Some(*self.current.frame());
        self.index = (self.index + 1) % self.factories.len();
        self.current = self.factories[self.index]();
        self.last.as_ref().expect("just stored")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::DisplaySelect;
    use crate::input::key;

    /// A test app: counts its ticks, exposes the count through the
    /// frame's sub-backdrop, advances after `ticks`.
    struct Counting {
        ticks: u32,
        ran: u32,
        frame: LogicalFrame,
    }

    impl Counting {
        fn factory(ticks: u32) -> Box<dyn Fn() -> Box<dyn App>> {
            Box::new(move || {
                Box::new(Counting {
                    ticks,
                    ran: 0,
                    frame: LogicalFrame::default(),
                })
            })
        }
    }

    impl App for Counting {
        fn tick(&mut self, _frame: crate::Frame, _input: Input) {
            self.ran += 1;
            self.frame.sub.backdrop = self.ran as u16;
            self.frame.display = DisplaySelect::SubOnTop;
        }

        fn frame(&self) -> &LogicalFrame {
            &self.frame
        }

        fn next(&self) -> ChainNext {
            if self.ran >= self.ticks {
                ChainNext::Advance
            } else {
                ChainNext::Stay
            }
        }
    }

    #[test]
    fn chain_runs_each_app_until_it_advances() {
        let mut chain = BootChain::new(vec![Counting::factory(2), Counting::factory(1)]);
        let ticks = (0..5)
            .map(|i| {
                chain
                    .tick(crate::Frame { index: i }, Input::default())
                    .sub
                    .backdrop
            })
            .collect::<Vec<_>>();
        // First app: runs (1), finishes (2) — the advancing tick still
        // returns its last frame. Second app: its only tick (1), which
        // also advances, so the cycle wraps to the first factory. The
        // wrapped-first instance: first tick (1), second tick (2).
        assert_eq!(ticks, [1, 2, 1, 1, 2]);
    }

    #[test]
    fn chain_cycles_with_fresh_instances() {
        // A single-app chain restarts itself: the factory must run
        // again on every wrap. The app marks its instance into the
        // shared counter at construction.
        let instantiations = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let counter = instantiations.clone();
        let factory: Box<dyn Fn() -> Box<dyn App>> = Box::new(move || {
            counter.set(counter.get() + 1);
            Counting::factory(1)()
        });
        let mut chain = BootChain::new(vec![factory]);
        for _ in 0..4 {
            chain.tick(crate::Frame { index: 0 }, Input::default());
        }
        // One construction at BootChain::new plus one per advancing
        // tick — the app advances every tick (ticks == 1).
        assert_eq!(instantiations.get(), 5);
    }

    #[test]
    fn apps_see_their_input() {
        // The skip logic of the copyright beat: A or START pressed.
        struct Skippable {
            skipped: bool,
            frame: LogicalFrame,
        }
        impl App for Skippable {
            fn tick(&mut self, _frame: crate::Frame, input: Input) {
                if input.keys.any(key::A | key::START) {
                    self.skipped = true;
                    self.frame.main.backdrop = 1;
                }
            }
            fn frame(&self) -> &LogicalFrame {
                &self.frame
            }
            fn next(&self) -> ChainNext {
                if self.skipped {
                    ChainNext::Advance
                } else {
                    ChainNext::Stay
                }
            }
        }
        let factory: Box<dyn Fn() -> Box<dyn App>> = Box::new(|| {
            Box::new(Skippable {
                skipped: false,
                frame: LogicalFrame::default(),
            })
        });
        let mut chain = BootChain::new(vec![factory]);
        let f = chain.tick(
            crate::Frame { index: 0 },
            Input {
                keys: crate::input::Keys(key::A),
                touch: None,
            },
        );
        assert_eq!(f.main.backdrop, 1, "the skip shows in the frame");
        // The advance swapped in a fresh, un-skipped instance: its
        // next tick shows the cleared state, not the skip.
        let f = chain.tick(crate::Frame { index: 1 }, Input::default());
        assert_eq!(f.main.backdrop, 0, "a fresh instance after the advance");
    }
}
