//! The game-state machine — pret's main-overlay chain, boot to the
//! player's bedroom (PLAN.md Phase 4, step 5).
//!
//! The game's "state machine" is not a switch in one file: it is the
//! chain of *main overlays*. `NitroMain` registers one overlay at a
//! time (`RegisterMainOverlay`, `src/main.c`), and each overlay's
//! exit registers the next. [`Game`] ports that chain — each
//! [`GameState`] is one main overlay, and each transition is the
//! `RegisterMainOverlay` call in the finishing overlay's exit:
//!
//! | state | pret's overlay | next (citation) |
//! |---|---|---|
//! | [`IntroMovie`] | `gApplication_IntroMovie` | title — `intro_movie.c:163` |
//! | [`Title`] | `gApplication_TitleScreen` | menu/timeout → `title_screen.c:248`/`:255` |
//! | [`CheckSave`] | `gApplication_CheckSave` | the main menu — `check_savedata.c:188` |
//! | [`MainMenu`] | `gApp_MainMenu` (OVY_74) | by exit — `main_menu.c:1482-1518` |
//! | [`NewGameInit`] | `ov36_App_MainMenu_SelectOption_NewGame` | Oak's speech — `overlay_36.c:112` |
//! | [`OakSpeech`] | `gApplication_OakSpeech` | post-Oak init — `oaks_speech.c:650` |
//! | [`AfterOakSpeech`] | `ov36_App_InitGameState_AfterOakSpeech` | the field — `overlay_36.c:138` |
//! | [`Bedroom`] | `gApplication_NewGameFieldsys` | terminal this phase (Phase 5 overworld) |
//! | [`Continue`] | the menu's app leaves | terminal this phase |
//!
//! [`IntroMovie`]: GameState::IntroMovie
//! [`Title`]: GameState::Title
//! [`CheckSave`]: GameState::CheckSave
//! [`MainMenu`]: GameState::MainMenu
//! [`NewGameInit`]: GameState::NewGameInit
//! [`OakSpeech`]: GameState::OakSpeech
//! [`AfterOakSpeech`]: GameState::AfterOakSpeech
//! [`Bedroom`]: GameState::Bedroom
//! [`Continue`]: GameState::Continue
//!
//! **Boot order** is `NitroMain`'s: probe the card backup
//! (`SaveData_New` — the card passed to [`Game::new`], with
//! [`SaveData::parse`](crate::save::SaveData::parse)'s outcome mapped
//! onto the status flags below), pin the RTC clock
//! (`GF_InitRTCWork` — the [`RtcDateTime`](crate::rtc::RtcDateTime)
//! argument), seed both generators (`InitializeMainRNG`, with the
//! vblank counter at 0), then register the intro.
//!
//! **The vblank counter is the tick index.** `NitroMain`'s loop
//! increments it once per frame and `InitializeMainRNG` reads it, so
//! the re-seed points (`ov36`'s inits run it again) are deterministic
//! functions of the frame they run on. The `ov36` overlays are
//! *constructed* on the frame after the previous one exited, so the
//! machine re-seeds on the entering state's **first tick**, with that
//! tick's index — the [`Frame`](crate::Frame) the caller hands it.
//!
//! **Scene ports pending** (same step, later landings): the naming
//! screen nested inside Oak's speech is the ported [`OakSpeech`]'s
//! one seam — the machine forwards
//! [`Game::deliver_naming_result`](Self::deliver_naming_result) to
//! it while it runs, the naming scene itself arriving with its port —
//! and the post-Oak `ov36` pass remains a one-tick [`StubScene`] here.
//! The machine ports the overlay *boundaries* faithfully (who runs
//! after whom, what re-seeds, what the status flags say) and the
//! scenes' inner frames arrive with their ports, plugged into the
//! same [`GameState`] slots. Likewise deferred, until the structured
//! save blocks land: the new-game save mutations
//! (`NewGame_InitSaveData`: money 3000, position to the player's
//! room, the fishing record, flag 960) and the post-Oak
//! initialization (`InitGameStateAfterOakSpeech_Internal`: trainer
//! ID, avatar, Safari Zone reset, the friend-group/Kenya mail, the
//! ten Pokewalker seeds — all fed by the MT generator the machine
//! seeds and holds).
//!
//! The title screen's CLEARSAVE and MIC_TEST exits
//! (`title_screen.c:250-259` — a key combo on the title screen, the
//! GameFreak mic test) are outside the plan's flow and unmodeled. The
//! menu's CONTINUE and app leaves (`main_menu.c:1488-1515`) register
//! overlays later phases port; the machine routes them through
//! [`GameState::Continue`], a cleared-frame leaf that never advances.

use std::fmt;
use std::sync::{Arc, Mutex};

use crate::assets::AssetStore;
use crate::frame::LogicalFrame;
use crate::input::Input;
use crate::rng::{Lcrng, Mt19937};
use crate::rtc::RtcDateTime;
use crate::save::{SaveData, SaveError};

use super::check_save::CheckSave;
use super::intro_copyright::IntroCopyright;
use super::main_menu::{MainMenu, MainMenuExit};
use super::oak_speech::OakSpeech;
use super::title_screen::{TitleExit, TitleScreen};
use super::{App, ChainNext};
use crate::text::string::GameString;

/// `Save_GetStatusFlags` bit 0 (`src/save.c`): the load fell back to
/// the previous generation (`LOAD_STATUS_SLOT_FAIL`) — CheckSave
/// warns "The save file is corrupted. The previous save file will be
/// loaded." (`msg_0229_00000`) before the menu.
pub const SAVE_STATUS_SLOT_DEGRADED: u32 = 1 << 0;
/// `Save_GetStatusFlags` bit 1: nothing loadable
/// (`LOAD_STATUS_TOTAL_FAIL`) — the game starts fresh and CheckSave
/// warns "The save file will be erased due to corruption or damage."
/// (`msg_0229_00001`).
pub const SAVE_STATUS_TOTAL_FAIL: u32 = 1 << 1;

// Bits 2–5 of `Save_GetStatusFlags` — the Battle Hall and Battle
// Video degraded/corrupt pairs (`Save_CheckFrontierData`) — are
// not computed yet: they need the structured frontier blocks. The
// real CheckSave port reads them off this same field.

/// One main overlay of the boot chain — where
/// `RegisterMainOverlay` (`src/main.c`) points between states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameState {
    /// The intro movie (this port: the copyright beat).
    IntroMovie,
    /// The title screen.
    Title,
    /// The save-check app: status-flag warnings, then the menu.
    CheckSave,
    /// The post-title menu (CONTINUE / NEW GAME / …).
    MainMenu,
    /// The new-game choice's `ov36` overlay — one exec pass of save
    /// mutations, then Oak.
    NewGameInit,
    /// Oak's speech, the gender pick, and the nested naming screen.
    OakSpeech,
    /// The post-Oak `ov36` overlay — the rest of game-state init,
    /// then the field.
    AfterOakSpeech,
    /// The new-game field entry: the player's bedroom. Phase 4's end
    /// state — the overworld is Phase 5, so the machine holds here
    /// (a cleared frame).
    Bedroom,
    /// The menu's CONTINUE and app leaves (`main_menu.c:1488-1515` —
    /// the Pokewalker, the mystery-gift and migrate apps, the Wii
    /// connect, the WFC setup, the Wii message settings): overlays
    /// later phases port. A cleared-frame leaf that never advances,
    /// like [`Self::Bedroom`].
    Continue,
}

/// A construction failure with no game flow to route to: the card
/// blob is not a card backup at all — the one parse outcome pret has
/// no screen for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameError {
    /// The blob's size is not the 512-KiB card backup.
    NotACardBackup {
        /// The size found, in bytes.
        got: usize,
    },
}

impl fmt::Display for GameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GameError::NotACardBackup { got } => {
                write!(f, "the card blob is not a card backup: got {got} bytes")
            }
        }
    }
}

impl std::error::Error for GameError {}

/// The one-tick stand-in for a scene whose port lands later in the
/// step (the save-check banner, the menu, Oak's speech, the `ov36`
/// init passes): a cleared frame, advancing on its first tick — the
/// machine keeps the overlay boundary, the scene brings its frames.
struct StubScene {
    /// The cleared frame every tick shows.
    frame: LogicalFrame,
    /// Whether the first tick has run.
    ticked: bool,
}

impl App for StubScene {
    fn tick(&mut self, _frame: crate::Frame, _input: Input) {
        self.ticked = true;
    }

    fn frame(&self) -> &LogicalFrame {
        &self.frame
    }

    fn next(&self) -> ChainNext {
        if self.ticked {
            ChainNext::Advance
        } else {
            ChainNext::Stay
        }
    }
}

/// The overlay the machine is running — one scene per state, held
/// concretely so the state's exit contract is readable without
/// downcasting.
enum Scene {
    /// `gApplication_IntroMovie` — the copyright beat app.
    Intro(IntroCopyright),
    /// `gApplication_TitleScreen` — the title screen app.
    Title(TitleScreen),
    /// `gApplication_CheckSave` — the save-check app.
    CheckSave(CheckSave),
    /// `gApp_MainMenu` — the main menu app.
    MainMenu(MainMenu),
    /// `gApplication_OakSpeech` — Oak's speech app.
    Oak(OakSpeech),
    /// The pending scene ports (see [`GameState`]'s per-state docs).
    Stub(StubScene),
    /// The bedroom, this phase's terminal state: a cleared frame,
    /// forever (the fieldsys renders the room in Phase 5).
    Bedroom(LogicalFrame),
    /// The menu's CONTINUE and app leaves, this phase's other
    /// terminal state: a cleared frame, forever.
    Continue(LogicalFrame),
}

impl Scene {
    fn tick(&mut self, frame: crate::Frame, input: Input) {
        match self {
            Scene::Intro(app) => app.tick(frame, input),
            Scene::Title(app) => app.tick(frame, input),
            Scene::CheckSave(app) => app.tick(frame, input),
            Scene::MainMenu(app) => app.tick(frame, input),
            Scene::Oak(app) => app.tick(frame, input),
            Scene::Stub(stub) => stub.tick(frame, input),
            Scene::Bedroom(_) | Scene::Continue(_) => {}
        }
    }

    fn frame(&self) -> &LogicalFrame {
        match self {
            Scene::Intro(app) => app.frame(),
            Scene::Title(app) => app.frame(),
            Scene::CheckSave(app) => app.frame(),
            Scene::MainMenu(app) => app.frame(),
            Scene::Oak(app) => app.frame(),
            Scene::Stub(stub) => stub.frame(),
            Scene::Bedroom(frame) | Scene::Continue(frame) => frame,
        }
    }

    fn next(&self) -> ChainNext {
        match self {
            Scene::Intro(app) => app.next(),
            Scene::Title(app) => app.next(),
            Scene::CheckSave(app) => app.next(),
            Scene::MainMenu(app) => app.next(),
            Scene::Oak(app) => app.next(),
            Scene::Stub(stub) => stub.next(),
            Scene::Bedroom(_) | Scene::Continue(_) => ChainNext::Stay,
        }
    }
}

/// The game: pret's main-overlay chain, one overlay at a time, boot
/// to the player's bedroom.
///
/// The tick contract mirrors [`BootChain`](super::BootChain)'s: the
/// tick that advances returns the *finishing* state's last frame,
/// the next tick belongs to the new state, and a state is
/// constructed fresh at every entry (a timeout cycle back to the
/// intro is a true reset, as the overlay reload is).
pub struct Game {
    /// The pinned asset store both boot scenes load from (shared
    /// with the presenter, as `boot_chain`'s is).
    store: Arc<Mutex<AssetStore>>,
    /// The overlay currently running.
    state: GameState,
    /// The running overlay's scene.
    scene: Scene,
    /// The parsed card backup — `None` is a fresh region (no save,
    /// or the erased TOTAL_FAIL one).
    save: Option<SaveData>,
    /// `Save_GetStatusFlags` (`src/save.c:88-118`): bit 0 the
    /// degraded slot, bit 1 the erased one. CheckSave reads this.
    save_status_flags: u32,
    /// The pinned clock (`GF_InitRTCWork` over hardware).
    rtc: RtcDateTime,
    /// The main field RNG, seeded by `InitializeMainRNG`.
    lcrng: Lcrng,
    /// The boot-seeded MT generator, seeded alongside it (its first
    /// engine draws are the deferred post-Oak init's).
    mtrng: Mt19937,
    /// Whether the entering state's first tick runs
    /// `InitializeMainRNG` (the `ov36` inits do; see the module doc).
    pending_reseed: bool,
    /// The finishing menu's exit, kept across the advance past the
    /// menu (`MainMenu_QueueSelectedApp`'s pick — readable once the
    /// machine has moved on, as the registered overlay's identity).
    menu_exit: Option<MainMenuExit>,
    /// The finishing state's last frame, kept across a transition so
    /// the advancing tick still returns it.
    last: Option<LogicalFrame>,
}

impl Game {
    /// Boots the game: probe the card, pin the clock, seed the
    /// generators, register the intro — `NitroMain`'s order.
    ///
    /// `card` is the card backup; `None` models no card data — a
    /// blank (or wiped) card, pret's `LOAD_STATUS_NOT_EXIST` fresh
    /// start. The parse outcome maps onto
    /// [`Self::save_status_flags`] exactly as `save.c:88-118` sets
    /// `statusFlags`.
    ///
    /// # Errors
    /// [`GameError::NotACardBackup`] when the blob is not a card
    /// backup — no game flow exists for it. The two other parse
    /// failures are *routed*, not errors: a blank card boots a new
    /// game silently, a corrupt one boots a new game behind bit 1's
    /// warning.
    ///
    /// # Panics
    /// Panics when an asset member fails to load — the pinned-store
    /// convention of [`boot_chain`](super::boot_chain): a broken
    /// member is a broken table, not a runtime condition.
    pub fn new(
        store: Arc<Mutex<AssetStore>>,
        card: Option<&[u8]>,
        rtc: RtcDateTime,
    ) -> Result<Self, GameError> {
        let (save, save_status_flags) = match card {
            None => (None, 0),
            Some(blob) => match SaveData::parse(blob) {
                Ok(data) => {
                    let flags = if data.slot_degraded() {
                        SAVE_STATUS_SLOT_DEGRADED
                    } else {
                        0
                    };
                    (Some(data), flags)
                }
                // NOT_EXIST: a fresh region, no warning (save.c's
                // fallthrough sets no flags).
                Err(SaveError::NoSaveData) => (None, 0),
                // TOTAL_FAIL: bit 1, then the fresh region — the
                // game continues behind CheckSave's erase warning.
                Err(SaveError::Corrupt) => (None, SAVE_STATUS_TOTAL_FAIL),
                Err(SaveError::NotACardBackup { got }) => {
                    return Err(GameError::NotACardBackup { got })
                }
            },
        };
        let mut game = Self {
            scene: Scene::Intro(
                IntroCopyright::load(&store)
                    .expect("the pinned ROM's copyright-beat members load"),
            ),
            store,
            state: GameState::IntroMovie,
            save,
            save_status_flags,
            rtc,
            lcrng: Lcrng::new(0),
            mtrng: Mt19937::uninitialized(),
            pending_reseed: false,
            menu_exit: None,
            last: None,
        };
        // InitializeMainRNG, with the loop not yet run: counter 0.
        game.initialize_main_rng(0);
        Ok(game)
    }

    /// The overlay currently running.
    #[must_use]
    pub fn state(&self) -> GameState {
        self.state
    }

    /// The loaded card backup, when one exists — `SaveData_New`'s
    /// parsed result (the menu's CONTINUE edge reads its presence).
    #[must_use]
    pub fn save(&self) -> Option<&SaveData> {
        self.save.as_ref()
    }

    /// `Save_GetStatusFlags` — the CheckSave warnings, bit by bit.
    #[must_use]
    pub fn save_status_flags(&self) -> u32 {
        self.save_status_flags
    }

    /// `Save_FileExists`: a loadable save generation is on the card.
    #[must_use]
    pub fn save_file_exists(&self) -> bool {
        self.save.is_some()
    }

    /// The pinned RTC clock (`GF_RTC_CopyDateTime`'s source) — what
    /// Oak's time-of-day greeting reads.
    #[must_use]
    pub fn rtc(&self) -> RtcDateTime {
        self.rtc
    }

    /// The main field RNG (`GetLCRNGSeed`'s static), as seeded by the
    /// last `InitializeMainRNG`.
    #[must_use]
    pub fn lcrng(&self) -> &Lcrng {
        &self.lcrng
    }

    /// The boot-seeded MT generator (`sMTRNG_State`'s home), as
    /// seeded by the last `InitializeMainRNG`.
    #[must_use]
    pub fn mtrng(&self) -> &Mt19937 {
        &self.mtrng
    }

    /// The finishing menu's exit —
    /// `MainMenu_QueueSelectedApp`'s pick, read off the menu before
    /// the advance replaced it. `None` until the machine's first menu
    /// finishes; every later menu overwrites it.
    #[must_use]
    pub fn menu_exit(&self) -> Option<MainMenuExit> {
        self.menu_exit
    }

    /// The naming screen's delivered result, forwarded to Oak's
    /// speech while it runs — the speech's own overlay seam (its
    /// `OverlayManager_Run` reporting TRUE, `oaks_speech.c:622-626`).
    ///
    /// # Panics
    /// Panics outside [`GameState::OakSpeech`] — only the speech's
    /// nested naming screen produces a result.
    pub fn deliver_naming_result(&mut self, name: GameString) {
        let Scene::Oak(oak) = &mut self.scene else {
            unreachable!("only Oak's speech takes a naming result");
        };
        oak.deliver_naming_result(name);
    }

    /// The current scene's logical frame (the running overlay's
    /// frame; on the tick after a transition, the *new* scene's
    /// cleared state — [`Self::tick`]'s return carried the finishing
    /// frame across).
    #[must_use]
    pub fn frame(&self) -> &LogicalFrame {
        self.scene.frame()
    }

    /// Advances the game one tick: the running overlay's scene ticks,
    /// and when it finishes the machine applies the finishing
    /// overlay's `RegisterMainOverlay`.
    ///
    /// Returns the logical frame this tick produced: the scene's
    /// frame, or on an advancing tick the finishing state's last
    /// frame — the next tick belongs to the new state.
    pub fn tick(&mut self, frame: crate::Frame, input: Input) -> &LogicalFrame {
        if self.pending_reseed {
            // The ov36 inits run InitializeMainRNG at overlay
            // construction — one frame after the exit — so the seed
            // reads this tick's frame index.
            self.pending_reseed = false;
            self.initialize_main_rng(frame.index);
        }
        self.scene.tick(frame, input);
        if self.scene.next() == ChainNext::Advance {
            // Retain the finishing frame, then swap in the next
            // state's scene, constructed fresh.
            self.last = Some(self.scene.frame().clone());
            self.advance();
            return self.last.as_ref().expect("just stored");
        }
        self.scene.frame()
    }

    /// `InitializeMainRNG` (`src/main.c`): one seed from the RTC —
    /// the pinned clock plus the vblank counter — drives both
    /// generators.
    fn initialize_main_rng(&mut self, vblank_counter: u32) {
        let seed = self.rtc.rng_seed(vblank_counter);
        self.lcrng.set_seed(seed);
        self.mtrng.set_seed(seed);
    }

    /// The finishing overlay's exit: which state it registered, and
    /// the new scene — each transition cited per-state in the module
    /// table.
    fn advance(&mut self) {
        if self.state == GameState::MainMenu {
            // Keep the menu's pick past the advance that replaces
            // the scene — `MainMenu_QueueSelectedApp`'s argument.
            let Scene::MainMenu(menu) = &self.scene else {
                unreachable!("the menu state runs the menu scene")
            };
            self.menu_exit = Some(
                menu.exit()
                    .expect("the menu sets its exit before advancing"),
            );
        }
        self.state = self.next_state();
        // The ov36 overlays re-seed at init: the new-game pass and
        // the post-Oak pass (overlay_36.c:96, :120). Their
        // CONTINUE sibling (:145) joins with the menu's port.
        self.pending_reseed =
            matches!(self.state, GameState::NewGameInit | GameState::AfterOakSpeech);
        self.scene = self.scene_for(self.state);
    }

    /// The registered-next table — every overlay's
    /// `RegisterMainOverlay` call, in chain order.
    fn next_state(&self) -> GameState {
        match self.state {
            // intro_movie.c:163.
            GameState::IntroMovie => GameState::Title,
            GameState::Title => {
                // title_screen.c's exit table: MENU → the save-check
                // (:248), TIMEOUT → the intro (:255).
                let Scene::Title(title) = &self.scene else {
                    unreachable!("the title state runs the title scene")
                };
                match title
                    .exit()
                    .expect("the title screen sets its exit mode before advancing")
                {
                    TitleExit::Menu => GameState::CheckSave,
                    TitleExit::Timeout => GameState::IntroMovie,
                }
            }
            // check_savedata.c:188.
            GameState::CheckSave => GameState::MainMenu,
            GameState::MainMenu => {
                // MainMenu_QueueSelectedApp's table (:1482-1518),
                // read off the stored pick. NEW GAME registers the
                // ov36 init (:1491); the title edge (:1518) is the
                // B pick's intro-title overlay; CONTINUE and the app
                // leaves (:1488, :1494-1515) register overlays later
                // phases port — the Continue leaf holds their place.
                match self
                    .menu_exit
                    .expect("advance stored the menu's exit first")
                {
                    MainMenuExit::NewGame => GameState::NewGameInit,
                    MainMenuExit::BackToTitle => GameState::Title,
                    MainMenuExit::Continue
                    | MainMenuExit::MysteryGift
                    | MainMenuExit::MigrateAgb
                    | MainMenuExit::ConnectToRanger
                    | MainMenuExit::ConnectToWii
                    | MainMenuExit::Wfc
                    | MainMenuExit::Pokewalker
                    | MainMenuExit::WiiSettings => GameState::Continue,
                }
            }
            // overlay_36.c:112.
            GameState::NewGameInit => GameState::OakSpeech,
            // oaks_speech.c:650.
            GameState::OakSpeech => GameState::AfterOakSpeech,
            // overlay_36.c:138 — gApplication_NewGameFieldsys.
            GameState::AfterOakSpeech => GameState::Bedroom,
            GameState::Bedroom | GameState::Continue => {
                unreachable!("the terminal states never advance")
            }
        }
    }

    /// The scene a state runs — the real app where ported, the
    /// one-tick stub where the port lands later in the step.
    fn scene_for(&self, state: GameState) -> Scene {
        match state {
            GameState::IntroMovie => Scene::Intro(
                IntroCopyright::load(&self.store)
                    .expect("the pinned ROM's copyright-beat members load"),
            ),
            GameState::Title => Scene::Title(
                TitleScreen::load(&self.store)
                    .expect("the pinned ROM's title-screen members load"),
            ),
            GameState::CheckSave => Scene::CheckSave(
                CheckSave::load(&self.store, self.save_status_flags)
                    .expect("the pinned ROM's save-check members load"),
            ),
            GameState::MainMenu => Scene::MainMenu(
                MainMenu::load(&self.store, self.save_file_exists())
                    .expect("the pinned ROM's main-menu members load"),
            ),
            GameState::OakSpeech => Scene::Oak(
                OakSpeech::load(&self.store, self.rtc)
                    .expect("the pinned ROM's Oak-speech members load"),
            ),
            GameState::NewGameInit | GameState::AfterOakSpeech => Scene::Stub(StubScene {
                frame: LogicalFrame::default(),
                ticked: false,
            }),
            GameState::Bedroom => Scene::Bedroom(LogicalFrame::default()),
            GameState::Continue => Scene::Continue(LogicalFrame::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_a_card_backup_reports_the_size() {
        // The one error display, as `SaveError`'s siblings do.
        let err = GameError::NotACardBackup { got: 1234 };
        assert_eq!(
            err.to_string(),
            "the card blob is not a card backup: got 1234 bytes"
        );
    }

    #[test]
    fn status_flag_bits_match_save_c() {
        // save.c:88-118 — bit 0 is the SLOT_FAIL warning, bit 1 the
        // TOTAL_FAIL one; the frontier pairs would start at bit 2.
        assert_eq!(SAVE_STATUS_SLOT_DEGRADED, 0b0001);
        assert_eq!(SAVE_STATUS_TOTAL_FAIL, 0b0010);
        assert_eq!(SAVE_STATUS_SLOT_DEGRADED & SAVE_STATUS_TOTAL_FAIL, 0);
    }
}