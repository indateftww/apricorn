//! Golden-hash rasterizer tests against a retail HeartGold (US) dump
//! — the pipeline pinned end to end, by SHA-1 only, never pixels.
//!
//! The dump points walk the boot chain's whole Phase 3 arc: the
//! copyright beat's hold (frame 0), the mid-fade identity (45 — the
//! register weights clamp at 16, so `ev` 8's EVA 23 shows the first
//! target unchanged over a black backdrop), the fade's black end
//! (100), the Game Freak logo (135, 200 — the bottom LCD's scroll
//! reset revealing the copyright map's upper half, the top LCD's
//! empty SUB map leaving black), and the title screen's settled
//! statics (320, 420, 1000 — the logo fade capped at local frame 63,
//! the deferred 3D BG0 leaving the bottom LCD black). The hashes are
//! the SHA-1 of each screen's raw RGBA bytes, the same digests
//! `apricorn-gfx-dump` prints, so the CLI, this test, and the future
//! harness all pin identical pixels.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer
//! supplies their own ROM dump).

use std::sync::{Arc, Mutex, OnceLock};

use apricorn_core::Frame;
use apricorn_core::app::boot_chain;
use apricorn_core::assets::AssetStore;
use apricorn_core::frame::DisplaySelect;
use apricorn_core::input::Input;
use sha1::{Digest, Sha1};

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// The dump points: global frame index → (top LCD, bottom LCD) SHA-1
/// of the rendered 256×192 RGBA screen.
///
/// Regenerate with
/// `apricorn-gfx-dump --rom hg_usa.nds --frame … --out out/frames`
/// and review the PNGs before changing this table — the golden hashes
/// are the contract, and a change here is a deliberate decision, not
/// a test fix.
const GOLDEN: &[(u32, &str, &str)] = &[
    // The copyright beat: the bottom LCD's scrolled copyright text
    // under the un-faded blend; the top LCD's blank cover.
    (
        0,
        "5a3d8dbf6ad60ee27c7aeb8b1edd9f567df372ec",
        "6328dca89ec034974f4cd2d6bc45019731ce06e3",
    ),
    // Mid-fade (counter 16, `ev` 8): EVA 23 clamps to 16 — the
    // identity over the black backdrop, so both screens match
    // frame 0 pixel for pixel.
    (
        45,
        "5a3d8dbf6ad60ee27c7aeb8b1edd9f567df372ec",
        "6328dca89ec034974f4cd2d6bc45019731ce06e3",
    ),
    // The fade's end (EVA 0, EBV 31→16): both engines black.
    (
        100,
        "31c8daa3770fde1d76332d8aac4f53357adc1ce6",
        "31c8daa3770fde1d76332d8aac4f53357adc1ce6",
    ),
    // The Game Freak logo: the bottom LCD's scroll reset (the
    // copyright map's upper half), the top LCD empty (SUB BG1's map
    // is the all-transparent tile) over the black backdrop.
    (
        135,
        "31c8daa3770fde1d76332d8aac4f53357adc1ce6",
        "88440023b5976b12026d1e1f62c0da980580c336",
    ),
    // The logo's 110-frame hold: unchanged.
    (
        200,
        "31c8daa3770fde1d76332d8aac4f53357adc1ce6",
        "88440023b5976b12026d1e1f62c0da980580c336",
    ),
    // The title screen, logo fade long since capped: the top LCD's
    // SUB statics; the bottom LCD black — MAIN's only content is the
    // deferred 3D BG0 and the cleared flash window.
    (
        320,
        "075c603f7cce914343d0b8d9fbb48e3497325ae2",
        "31c8daa3770fde1d76332d8aac4f53357adc1ce6",
    ),
    // Title statics are settled — the flash toggles a transparent
    // window — so 420 and 1000 match 320.
    (
        420,
        "075c603f7cce914343d0b8d9fbb48e3497325ae2",
        "31c8daa3770fde1d76332d8aac4f53357adc1ce6",
    ),
    (
        1000,
        "075c603f7cce914343d0b8d9fbb48e3497325ae2",
        "31c8daa3770fde1d76332d8aac4f53357adc1ce6",
    ),
];

/// The shared, once-opened pinned dump (SHA-1 gate inside), or `None`
/// to skip silently.
fn store() -> Option<Arc<Mutex<AssetStore>>> {
    static STORE: OnceLock<Option<Arc<Mutex<AssetStore>>>> = OnceLock::new();
    STORE
        .get_or_init(|| match AssetStore::open(std::path::Path::new(ROM_PATH)) {
            Ok(store) => Some(Arc::new(Mutex::new(store))),
            Err(_) => {
                eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
                None
            }
        })
        .clone()
}

/// One screen's SHA-1 over its raw RGBA bytes — the dump CLI's digest.
fn sha1_hex(screen: &apricorn_gfx::ScreenBuffer) -> String {
    let digest = Sha1::digest(screen.as_rgba().as_flattened());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[test]
fn golden_boot_chain_raster_hashes() {
    let Some(store) = store() else {
        return;
    };
    let mut chain = boot_chain(Arc::clone(&store));
    let last = GOLDEN[GOLDEN.len() - 1].0;

    for index in 0..=last {
        let frame = chain.tick(Frame { index }, Input::default());
        let Ok(at) = GOLDEN.binary_search_by_key(&index, |(i, _, _)| *i) else {
            continue;
        };
        let (i, top_hash, bottom_hash) = GOLDEN[at];
        let screens = {
            let guard = store
                .lock()
                .expect("the asset store is only locked at dump frames");
            apricorn_gfx::render(frame, &*guard)
        };
        let (top, bottom) = match frame.display {
            DisplaySelect::MainOnTop => (&screens[0], &screens[1]),
            DisplaySelect::SubOnTop => (&screens[1], &screens[0]),
        };
        assert_eq!(sha1_hex(top), top_hash, "frame {i} top LCD");
        assert_eq!(sha1_hex(bottom), bottom_hash, "frame {i} bottom LCD");
    }
}
