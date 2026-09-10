//! The apricorn desktop shell — PLAN.md Phase 3, step 8.
//!
//! The topology: this crate is the window (winit) + present (wgpu)
//! shell around the headless core. Every tick runs
//! [`apricorn_core::app::game::Game`]'s machine — the boot chain the
//! title's timeout cycles, the save-check menu, and the new-game
//! walk into Oak's speech (the naming screen past it is the next
//! step, so the speech ends by stalling at the name overlay) — and
//! every present rasterizes the machine's logical frame with
//! [`apricorn_gfx::render`] and uploads it — the same pipeline the
//! dump CLI and the golden-hash tests pin, so the window shows
//! exactly what the hashes certify. Nothing here can change engine
//! behavior: the wall clock only paces ([`runner::Pacer`], 59.8268
//! Hz), never feeds state, and the machine runs on the tests' frozen
//! clock (a real-RTC read is future work).
//!
//! Input maps the keyboard onto the `REG_KEYXY` bits (the layout the
//! engine consumes): A/B/X/Y on their same-letter keys, the arrows as
//! the D-pad, Enter as START, Shift as SELECT, and L/R on their
//! same-letter keys. Touch input is deferred with the touch model —
//! every scene so far takes its pad path. Escape closes the window.
//!
//! Usage: `cargo run -p apricorn-desktop [--rom <path>]` — the ROM
//! defaults to `hg_usa.nds` in the working directory (repo root).

mod presenter;
mod runner;

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use apricorn_core::Frame;
use apricorn_core::app::game::Game;
use apricorn_core::assets::AssetStore;
use apricorn_core::frame::{DisplaySelect, LogicalFrame};
use apricorn_core::input::{Input, Keys, key};
use apricorn_core::rtc::RtcDateTime;
use presenter::Presenter;
use runner::Pacer;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// The frozen clock the pinned tests run — HG's US release date at
/// noon, so the window plays exactly the walk `apps_hg` certifies
/// (the noon greeting, the same seeds).
fn frozen_rtc() -> RtcDateTime {
    RtcDateTime::new(2010, 3, 14, 0, 12, 0, 0)
}

/// The keyboard → `REG_KEYXY` mapping. Physical-key based (position,
/// not glyph) so it survives any layout the OS applies.
fn key_bit(code: KeyCode) -> Option<u16> {
    let bit = match code {
        KeyCode::KeyA => key::A,
        KeyCode::KeyB => key::B,
        KeyCode::KeyX => key::X,
        KeyCode::KeyY => key::Y,
        KeyCode::ArrowUp => key::UP,
        KeyCode::ArrowDown => key::DOWN,
        KeyCode::ArrowLeft => key::LEFT,
        KeyCode::ArrowRight => key::RIGHT,
        KeyCode::Enter | KeyCode::NumpadEnter => key::START,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => key::SELECT,
        KeyCode::KeyL => key::L,
        KeyCode::KeyR => key::R,
        _ => return None,
    };
    Some(bit)
}

/// The shell's whole state: the machine it ticks, the input it feeds,
/// the window/presenter it draws through, and the pacer that decides
/// when to do each.
struct Shell {
    store: Arc<Mutex<AssetStore>>,
    game: Game,
    /// The `REG_KEYXY` bits currently held (bit set = held).
    keys: u16,
    window: Option<std::sync::Arc<Window>>,
    presenter: Option<Presenter>,
    /// The next tick's global frame index.
    frame_index: u32,
    /// The last logical frame the machine produced (its cleared state
    /// before the first tick — the machine is valid to draw at t=0).
    last_frame: LogicalFrame,
    pacer: Pacer,
}

impl Shell {
    fn new(store: Arc<Mutex<AssetStore>>) -> Self {
        // A blank card: the no-save path straight through the menu's
        // NEW GAME into Oak's speech.
        let game = Game::new(Arc::clone(&store), None, frozen_rtc())
            .expect("a blank card boots a new game");
        let last_frame = game.frame().clone();
        Self {
            store,
            game,
            keys: 0,
            window: None,
            presenter: None,
            frame_index: 0,
            last_frame,
            pacer: Pacer::new(Instant::now()),
        }
    }

    /// Advances the machine `ticks` times with the current input,
    /// remembering the last frame produced.
    fn tick(&mut self, ticks: u32) {
        if ticks == 0 {
            return;
        }
        let input = Input {
            keys: Keys(self.keys),
            touch: None,
        };
        for _ in 0..ticks {
            let frame = self.game.tick(Frame { index: self.frame_index }, input);
            self.last_frame = frame.clone();
            self.frame_index += 1;
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Rasterizes the last frame and presents it. Called on redraw
    /// only — new pixels arrive with new ticks.
    fn redraw(&mut self) {
        let Some(presenter) = &mut self.presenter else {
            return;
        };
        let screens = {
            let store = self
                .store
                .lock()
                .expect("the asset store lock is uncontended between ticks");
            apricorn_gfx::render(&self.last_frame, &*store)
        };
        let (top, bottom) = match self.last_frame.display {
            DisplaySelect::MainOnTop => (&screens[0], &screens[1]),
            DisplaySelect::SubOnTop => (&screens[1], &screens[0]),
        };
        presenter.present(top, bottom);
    }
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // 512×768 logical — the plan's "~256×384 logical" stacked pair
        // at a comfortable 2× default, letterboxed to any size.
        let attributes = Window::default_attributes()
            .with_title("apricorn")
            .with_inner_size(winit::dpi::LogicalSize::new(512.0, 768.0));
        let window = std::sync::Arc::new(
            event_loop
                .create_window(attributes)
                .expect("the desktop platform opens a window"),
        );
        // The surface (inside the presenter) shares the window's Arc;
        // the shell keeps its clone for redraw requests.
        self.presenter = Some(Presenter::new(std::sync::Arc::clone(&window)));
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Destroyed => {
                self.window = None;
                self.presenter = None;
            }
            WindowEvent::Resized(size) => {
                if let Some(presenter) = &mut self.presenter {
                    presenter.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let state = event.state;
                if let PhysicalKey::Code(code) = event.physical_key {
                    if code == KeyCode::Escape && state == ElementState::Pressed {
                        event_loop.exit();
                    } else if let Some(bit) = key_bit(code) {
                        match state {
                            ElementState::Pressed => self.keys |= bit,
                            ElementState::Released => self.keys &= !bit,
                        }
                    }
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let ticks = self.pacer.poll(Instant::now());
        self.tick(ticks);
        // Sleep until the next tick is due instead of spinning — the
        // Fifo present has already throttled redraws to vsync.
        let deadline = self.pacer.next_deadline(Instant::now());
        event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
    }
}

fn main() {
    // `--rom <path>` optional; the working directory's hg_usa.nds is
    // the default (cargo run runs from the repo root).
    let mut rom: Option<std::path::PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--rom" => match args.next() {
                Some(value) => rom = Some(std::path::PathBuf::from(value)),
                None => {
                    eprintln!("--rom needs a value");
                    eprintln!("usage: apricorn [--rom <path>]");
                    std::process::exit(64);
                }
            },
            other => {
                eprintln!("unknown argument {other:?}");
                eprintln!("usage: apricorn [--rom <path>]");
                std::process::exit(64);
            }
        }
    }
    let rom = rom.unwrap_or_else(|| Path::new("hg_usa.nds").to_path_buf());

    let store = match AssetStore::open(&rom) {
        Ok(store) => store,
        Err(err) => {
            eprintln!("cannot open {rom:?}: {err}");
            eprintln!("supply your own retail HeartGold (US) dump as hg_usa.nds");
            std::process::exit(1);
        }
    };

    let event_loop = EventLoop::new().expect("the desktop platform provides an event loop");
    event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now()));
    let mut shell = Shell::new(Arc::new(Mutex::new(store)));
    if let Err(err) = event_loop.run_app(&mut shell) {
        eprintln!("the event loop failed: {err}");
        std::process::exit(1);
    }
}
