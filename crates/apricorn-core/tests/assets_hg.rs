//! Integration tests: the boot asset loader against a retail
//! HeartGold (US) dump.
//!
//! `apricorn_core::assets`' tables pin *which* NARC members the
//! copyright beat and the title screen load (verified against pret
//! source and `.naix` member names). These tests decode every table
//! entry through the store — the same encode/parse path the converter
//! wrote — and pin each asset's observable shape (chunk kind, bpp,
//! tile count, screen dimensions, palette size, palette-bank usage) so
//! member drift or a decode regression fails loudly. The
//! [`AssetStore::open`] gate itself doubles as the SHA-1 pin: it
//! refuses any image that is not the dump the tables are valid for.
//!
//! Skips silently when `hg_usa.nds` is absent (each developer supplies
//! their own ROM dump).

use apricorn_core::assets::{AssetStore, copyright_beat, title_screen};
use apricorn_core::cache;
use apricorn_core::formats::CharMapping;
use apricorn_core::frame::AssetId;

const ROM_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds");

/// Opens the pinned dump, or `None` to skip silently.
fn store() -> Option<AssetStore> {
    match AssetStore::open(std::path::Path::new(ROM_PATH)) {
        Ok(store) => Some(store),
        Err(_) => {
            eprintln!("skipping: {ROM_PATH} not found (supply your own ROM dump)");
            None
        }
    }
}

/// The highest palette bank any text-screen entry references —
/// `0` for 8bpp screens (entries carry no bank) and for empty maps.
fn max_bank(screen: &cache::Screen) -> u16 {
    let entries = screen.entries();
    let mut max = 0;
    for pair in entries.chunks_exact(2) {
        max = max.max(u16::from_le_bytes([pair[0], pair[1]]) >> 12);
    }
    max
}

#[test]
fn copyright_beat_assets_decode() {
    let Some(mut store) = store() else {
        return;
    };
    let narc = copyright_beat::NARC;

    // MAIN BG0 — the copyright text (4bpp, 160 tiles, 2D mapping).
    let char = store
        .load_tiles(narc, copyright_beat::MAIN_BG0_CHAR)
        .expect("char");
    let tiles = store.tiles(char).expect("tile data");
    assert!(tiles.is_4bpp());
    assert_eq!(tiles.tile_count(), 160);
    assert_eq!(tiles.mapping(), CharMapping::TwoD);

    // MAIN BG0's screen — 256×256, 4bpp text format.
    let scr = store
        .load_screen(narc, copyright_beat::MAIN_BG0_SCREEN)
        .expect("screen");
    let screen = store.screen(scr).expect("screen data");
    assert_eq!((screen.width(), screen.height()), (256, 256));
    assert_eq!(screen.color_mode(), 0);
    assert_eq!(screen.screen_format(), 0);

    // SUB BG1 — the Game Freak logo (4bpp, 96 tiles) and its screen
    // (256×192 — a shorter map; the layer's bottom 8 tile rows fall
    // back to tile 0).
    let char = store
        .load_tiles(narc, copyright_beat::SUB_BG1_CHAR)
        .expect("char");
    let tiles = store.tiles(char).expect("tile data");
    assert!(tiles.is_4bpp());
    assert_eq!(tiles.tile_count(), 96);
    let scr = store
        .load_screen(narc, copyright_beat::SUB_BG1_SCREEN)
        .expect("screen");
    let screen = store.screen(scr).expect("screen data");
    assert_eq!((screen.width(), screen.height()), (256, 192));
    assert_eq!(screen.color_mode(), 0);

    // SUB BG0's screen — the blank cover sharing BG1's char block.
    let scr = store
        .load_screen(narc, copyright_beat::SUB_BG0_SCREEN)
        .expect("screen");
    let screen = store.screen(scr).expect("screen data");
    assert_eq!((screen.width(), screen.height()), (256, 256));
    assert_eq!(screen.color_mode(), 0);

    // The two palettes: 16-color files, 256 stored colors each, no
    // PMCP table — so placement is the identity.
    for member in [copyright_beat::SUB_PALETTE, copyright_beat::MAIN_PALETTE] {
        let pal = store.load_palette(narc, member).expect("palette");
        let palette = store.palette(pal).expect("palette data");
        assert!(palette.is_16_color());
        assert_eq!(palette.rgba().len(), 256);
        assert!(palette.pmcp().is_empty());
        assert_eq!(store.placed_palette(pal), Some(palette.rgba()));
    }

    // The scene loads 0x140 bytes of each palette — banks 0–9. Every
    // screen the beat can show must stay inside that range (the
    // sunrise layers included), or the partial load would matter.
    let loaded_banks = (copyright_beat::PAL_LOAD_BYTES / 2 / 16) as u16;
    let mut screens = vec![
        copyright_beat::MAIN_BG0_SCREEN,
        copyright_beat::SUB_BG1_SCREEN,
        copyright_beat::SUB_BG0_SCREEN,
    ];
    screens.extend([
        copyright_beat::SUB_BG3_SCREEN,
        copyright_beat::MAIN_BG3_SCREEN,
        copyright_beat::MAIN_BG2_SCREEN,
        copyright_beat::MAIN_BG1_SCREEN,
    ]);
    for member in screens {
        let scr = store.load_screen(narc, member).expect("screen");
        let screen = store.screen(scr).expect("screen data");
        assert_eq!(screen.screen_format(), 0);
        assert!(
            max_bank(screen) < loaded_banks,
            "{narc}#{member} uses bank {} beyond the 0x140 load",
            max_bank(screen)
        );
    }

    // The sunrise layers beyond the beat's scope: 4bpp, whole-tile
    // grids (256 and 512 tiles).
    let char = store
        .load_tiles(narc, copyright_beat::SUB_BG3_CHAR)
        .expect("char");
    assert_eq!(store.tiles(char).expect("tile data").tile_count(), 256);
    let char = store
        .load_tiles(narc, copyright_beat::MAIN_BG3_CHAR)
        .expect("char");
    assert_eq!(store.tiles(char).expect("tile data").tile_count(), 512);
}

#[test]
fn title_screen_assets_decode() {
    let Some(mut store) = store() else {
        return;
    };
    let narc = title_screen::NARC;

    // SUB BG1 — static art: 4bpp, 96 tiles, screen 256×192.
    let char = store
        .load_tiles(narc, title_screen::SUB_BG1_CHAR)
        .expect("char");
    let tiles = store.tiles(char).expect("tile data");
    assert!(tiles.is_4bpp());
    assert_eq!(tiles.tile_count(), 96);
    assert_eq!(tiles.mapping(), CharMapping::TwoD);
    let scr = store
        .load_screen(narc, title_screen::SUB_BG1_SCREEN)
        .expect("screen");
    let screen = store.screen(scr).expect("screen data");
    assert_eq!((screen.width(), screen.height()), (256, 192));
    assert_eq!(screen.color_mode(), 0);

    // SUB BG2 — the game logo: 8bpp, 480 tiles, screen 256×256.
    let char = store
        .load_tiles(narc, title_screen::SUB_BG2_CHAR)
        .expect("char");
    let tiles = store.tiles(char).expect("tile data");
    assert!(!tiles.is_4bpp());
    assert_eq!(tiles.tile_count(), 480);
    let scr = store
        .load_screen(narc, title_screen::SUB_BG2_SCREEN)
        .expect("screen");
    let screen = store.screen(scr).expect("screen data");
    assert_eq!((screen.width(), screen.height()), (256, 256));
    assert_eq!(screen.color_mode(), 1);

    // SUB BG3 — version art: 8bpp, 768 tiles, screen 256×192.
    let char = store
        .load_tiles(narc, title_screen::SUB_BG3_CHAR)
        .expect("char");
    let tiles = store.tiles(char).expect("tile data");
    assert!(!tiles.is_4bpp());
    assert_eq!(tiles.tile_count(), 768);
    let scr = store
        .load_screen(narc, title_screen::SUB_BG3_SCREEN)
        .expect("screen");
    let screen = store.screen(scr).expect("screen data");
    assert_eq!((screen.width(), screen.height()), (256, 192));
    assert_eq!(screen.color_mode(), 1);

    // The two palettes: 0x200-byte full loads. 4.pal is the 8bpp file
    // (SUB BG2/BG3 read all 256 colors); 13.pal is a 16-color file.
    // Neither carries a PMCP table — placement is the identity.
    let pal = store
        .load_palette(narc, title_screen::SUB_PALETTE)
        .expect("palette");
    let palette = store.palette(pal).expect("palette data");
    assert!(!palette.is_16_color());
    assert_eq!(palette.rgba().len(), 256);
    assert_eq!(store.placed_palette(pal), Some(palette.rgba()));
    let pal = store
        .load_palette(narc, title_screen::MAIN_PALETTE)
        .expect("palette");
    let palette = store.palette(pal).expect("palette data");
    assert!(palette.is_16_color());
    assert_eq!(palette.rgba().len(), 256);
    assert!(palette.pmcp().is_empty());
    assert_eq!(store.placed_palette(pal), Some(palette.rgba()));
}

#[test]
fn handles_assign_and_resolve_in_load_order() {
    let Some(mut store) = store() else {
        return;
    };
    // Loading in table order hands out sequential handles, and the
    // store resolves each handle to exactly the kind that produced it.
    let a = store
        .load_tiles(title_screen::NARC, title_screen::SUB_BG1_CHAR)
        .expect("tiles");
    let b = store
        .load_palette(title_screen::NARC, title_screen::SUB_PALETTE)
        .expect("palette");
    let c = store
        .load_screen(title_screen::NARC, title_screen::SUB_BG1_SCREEN)
        .expect("screen");
    assert_eq!(a, AssetId::FIRST);
    assert_eq!(b.index(), 1);
    assert_eq!(c.index(), 2);
    assert_eq!(store.len(), 3);
    assert!(store.tiles(b).is_none());
    assert!(store.palette(a).is_none());
    assert!(store.screen(b).is_none());
    // A handle past the loaded range resolves to nothing.
    assert!(store.tiles(AssetId::from_index(3)).is_none());
}

#[test]
fn table_drift_and_foreign_members_fail_loudly() {
    let Some(mut store) = store() else {
        return;
    };
    // A screen member loaded as tiles (and vice versa) is a table
    // drift — the store must refuse, not guess.
    assert!(
        store
            .load_tiles(title_screen::NARC, title_screen::SUB_BG1_SCREEN)
            .is_err()
    );
    assert!(
        store
            .load_screen(title_screen::NARC, title_screen::SUB_BG1_CHAR)
            .is_err()
    );
    // Missing archive and missing member.
    assert!(store.load_palette("no/such/narc", 0).is_err());
    assert!(store.load_palette(title_screen::NARC, 999).is_err());
}
