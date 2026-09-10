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
//! Oak launches [`NamingScreen`] as a nested overlay. The parent is
//! suspended until its result is ready; default-name selection draws from
//! this machine's LCRNG. Oak's confirmed identity is retained in
//! [`PlayerIdentity`] at the field handoff and written into [`NewGameData`].
//! The two `ov36` passes initialize the 42 save blocks and apply the
//! post-Oak trainer ID, avatar, Safari areas, mail and Pokewalker seeds.
//! Their scene boundaries are represented by one-tick [`InitPass`] values;
//! full original-overlay scheduler timing still needs differential coverage.
//! The bedroom holds ROM geometry and the chosen player texture; movement
//! and field scripts belong to Phase 5.
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
use crate::save::new_game::{ConsoleProfile, NewGameData};
use crate::save::{SaveData, SaveError};

use super::check_save::CheckSave;
use super::intro_copyright::IntroCopyright;
use super::main_menu::{MainMenu, MainMenuExit};
use super::naming::NamingScreen;
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
    /// (a static ROM-backed room).
    Bedroom,
    /// The menu's CONTINUE and app leaves (`main_menu.c:1488-1515` —
    /// the Pokewalker, the mystery-gift and migrate apps, the Wii
    /// connect, the WFC setup, the Wii message settings): overlays
    /// later phases port. A cleared-frame leaf that never advances,
    /// until the corresponding field/application port lands.
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

/// One-tick boundary for an ov36 initialization pass. `Game::tick`
/// applies its save mutations and RNG draws before ticking this value.
struct InitPass {
    /// The cleared frame every tick shows.
    frame: LogicalFrame,
    /// Whether the first tick has run.
    ticked: bool,
}

impl App for InitPass {
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
    /// New-game and post-Oak initialization boundaries.
    Init(InitPass),
    /// Static bedroom geometry and the confirmed player character.
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
            Scene::Init(stub) => stub.tick(frame, input),
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
            Scene::Init(stub) => stub.frame(),
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
            Scene::Init(stub) => stub.next(),
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
    /// The live region is separate from the loaded card until saved.
    new_game: Option<NewGameData>,
    /// The nested overlay launched by Oak; the parent is suspended.
    naming: Option<NamingScreen>,
    /// Oak's confirmed identity, retained across the field handoff.
    player: Option<PlayerIdentity>,
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
    /// engine draws include both ov36 initialization passes).
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

/// The player choices confirmed during Oak's introduction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerIdentity {
    /// Trainer name in the game's character encoding.
    pub name: GameString,
    /// Avatar selection (0 male, 1 female).
    pub gender: u8,
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
                    return Err(GameError::NotACardBackup { got });
                }
            },
        };
        let mut game = Self {
            new_game: None,
            naming: None,
            player: None,
            scene: Scene::Intro(
                IntroCopyright::load(&store).expect("the pinned ROM's copyright-beat members load"),
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

    /// The nested naming overlay, when active. The main state remains Oak.
    pub fn naming(&self) -> Option<&NamingScreen> {
        self.naming.as_ref()
    }

    /// Confirmed player choices, available after Oak's exit.
    pub fn player(&self) -> Option<&PlayerIdentity> {
        self.player.as_ref()
    }

    /// Initialized new-game save state, available after selecting NEW GAME.
    pub fn new_game_data(&self) -> Option<&NewGameData> {
        self.new_game.as_ref()
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

    /// The current scene's logical frame (the running overlay's
    /// frame; on the tick after a transition, the *new* scene's
    /// cleared state — [`Self::tick`]'s return carried the finishing
    /// frame across).
    #[must_use]
    pub fn frame(&self) -> &LogicalFrame {
        self.naming
            .as_ref()
            .map_or_else(|| self.scene.frame(), |n| n.frame())
    }

    /// Advances the game one tick: the running overlay's scene ticks,
    /// and when it finishes the machine applies the finishing
    /// overlay's `RegisterMainOverlay`.
    ///
    /// Returns the logical frame this tick produced: the scene's
    /// frame, or on an advancing tick the finishing state's last
    /// frame — the next tick belongs to the new state.
    pub fn tick(&mut self, frame: crate::Frame, input: Input) -> &LogicalFrame {
        if let Scene::Oak(oak) = &mut self.scene {
            if oak.naming_requested() && self.naming.is_none() {
                self.naming = Some(
                    NamingScreen::load(&self.store, oak.player_gender())
                        .expect("the pinned ROM's naming members load"),
                );
            }
            if let Some(naming) = &mut self.naming {
                naming.tick(frame, input);
                if naming.next() == ChainNext::Advance {
                    let name = naming.result(&mut self.lcrng).clone();
                    self.last = Some(naming.frame().clone());
                    oak.deliver_naming_result(name);
                    oak.tick(frame, input);
                    self.naming = None;
                    return self.last.as_ref().expect("naming's finishing frame");
                }
                return self.naming.as_ref().unwrap().frame();
            }
        }
        if self.pending_reseed {
            // The ov36 inits run InitializeMainRNG at overlay
            // construction — one frame after the exit — so the seed
            // reads this tick's frame index.
            self.pending_reseed = false;
            self.initialize_main_rng(frame.index);
            match self.state {
                GameState::NewGameInit => {
                    self.new_game = Some(
                        NewGameData::initialize(
                            &self.store.lock().expect("asset store"),
                            self.rtc,
                            frame.index,
                            &mut self.mtrng,
                            ConsoleProfile::default(),
                        )
                        .expect("the pinned ROM's new-game data loads"),
                    );
                }
                GameState::AfterOakSpeech => {
                    let player = self.player.as_ref().expect("Oak confirmed a player");
                    self.new_game
                        .as_mut()
                        .expect("NEW GAME initialized save data")
                        .finish_oak(
                            &player.name,
                            player.gender,
                            self.rtc,
                            &mut self.mtrng,
                            &mut self.lcrng,
                            ConsoleProfile::default(),
                        );
                }
                _ => {}
            }
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
        if let Scene::Oak(oak) = &self.scene {
            self.player = Some(PlayerIdentity {
                name: oak.player_name().clone(),
                gender: oak.player_gender(),
            });
        }
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
        self.pending_reseed = matches!(
            self.state,
            GameState::NewGameInit | GameState::AfterOakSpeech
        );
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
                TitleScreen::load(&self.store).expect("the pinned ROM's title-screen members load"),
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
            GameState::NewGameInit | GameState::AfterOakSpeech => Scene::Init(InitPass {
                frame: LogicalFrame::default(),
                ticked: false,
            }),
            GameState::Bedroom => {
                let mut frame = LogicalFrame::default();
                let gender = self.player.as_ref().expect("Oak confirmed a player").gender;
                frame.main.field = Some(Arc::new(
                    crate::field::FieldScene::bedroom(
                        &self.store.lock().expect("asset store"),
                        gender,
                    )
                    .expect("the pinned ROM's bedroom assets load"),
                ));
                Scene::Bedroom(frame)
            }
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
