//! Naming-screen raster smoke checks and optional local review PNGs.
use apricorn_core::app::{App, naming::NamingScreen};
use apricorn_core::assets::AssetStore;
use apricorn_core::input::{Input, Keys, key};
use std::sync::Mutex;

#[test]
fn naming_pages_render_from_rom() {
    let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds"));
    if !path.exists() {
        return;
    }
    let store = Mutex::new(AssetStore::open(path).unwrap());
    let mut app = NamingScreen::load(&store, 0).unwrap();
    let mut hashes = Vec::new();
    let mut name_box = None;
    for page in 0..4 {
        for index in 0..30 {
            app.tick(apricorn_core::Frame { index }, Input::default());
        }
        if page == 3 {
            for (x, y) in [(61, 108), (29, 89), (173, 108), (173, 108), (157, 108)] {
                app.tick(apricorn_core::Frame { index: 32 }, Input::default());
                app.tick(
                    apricorn_core::Frame { index: 33 },
                    Input {
                        keys: Keys::IDLE,
                        touch: Some(apricorn_core::input::Touch { x, y }),
                    },
                );
            }
            for index in 34..64 {
                app.tick(apricorn_core::Frame { index }, Input::default());
            }
            assert_eq!(app.entry(), &[311, 299, 318, 318, 317]);
        }
        let screens = apricorn_gfx::render(app.frame(), &*store.lock().unwrap());
        // Old keyboard pages must never cover the entry box above WIN0's
        // bottom edge, including after two page-switch animations.
        let region: Vec<_> = (24..39)
            .flat_map(|y| (80..168).map(move |x| (x, y)))
            .map(|(x, y)| screens[0].pixel(x, y))
            .collect();
        if page < 3 {
            if let Some(expected) = &name_box {
                assert_eq!(&region, expected);
            } else {
                name_box = Some(region);
            }
        } else {
            assert_ne!(&region, name_box.as_ref().unwrap());
        }
        // NamingScreen_SetBgModesAndInitBuffers + ToggleGfxPlanes: the
        // top LCD has a black backdrop and only its bottom prompt window.
        assert_eq!(screens[1].pixel(128, 64), [0, 0, 0, 255]);
        use sha1::{Digest, Sha1};
        let hash = format!("{:x}", Sha1::digest(screens[0].as_rgba().as_flattened()));
        assert!(
            !hashes.contains(&hash),
            "each page has distinct keyboard pixels"
        );
        hashes.push(hash);
        assert!(screens[0].as_rgba().iter().any(|p| *p != [0, 0, 0, 255]));
        if let Some(out) = std::env::var_os("APRICORN_RENDER_OUT") {
            std::fs::create_dir_all(&out).unwrap();
            let file = std::fs::File::create(
                std::path::PathBuf::from(out).join(format!("naming-{page}.png")),
            )
            .unwrap();
            let mut encoder = png::Encoder::new(file, 256, 384);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let pixels: Vec<u8> = [1, 0]
                .into_iter()
                .flat_map(|i| screens[i].as_rgba().as_flattened().iter().copied())
                .collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        }
        app.tick(
            apricorn_core::Frame { index: 31 },
            Input {
                keys: Keys(key::SELECT),
                touch: None,
            },
        );
    }
}
