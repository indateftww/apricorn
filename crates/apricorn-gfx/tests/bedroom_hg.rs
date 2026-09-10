//! ROM-gated field landing regression and optional review images.
use apricorn_core::{assets::AssetStore, field::FieldScene, frame::LogicalFrame};
use std::{path::Path, sync::Arc};

#[test]
fn bedroom_renders_real_models_and_both_players() {
    let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../hg_usa.nds"));
    if !path.exists() {
        return;
    }
    let store = AssetStore::open(path).unwrap();
    let mut images = Vec::new();
    for gender in 0..2 {
        let scene = FieldScene::bedroom(&store, gender).unwrap();
        assert_eq!(scene.map_id, 64);
        assert_eq!(scene.position, [6, 6]);
        assert!(scene.meshes.len() > 8);
        assert!(
            scene
                .meshes
                .iter()
                .map(|m| m.triangles.len())
                .sum::<usize>()
                > 250
        );
        let mut frame = LogicalFrame::default();
        frame.main.field = Some(Arc::new(scene));
        let screens = apricorn_gfx::render(&frame, &store);
        let pixels = screens[0].as_rgba();
        assert!(pixels.iter().filter(|c| c[..3] != [0, 0, 0]).count() > 15000);
        if let Ok(dir) = std::env::var("APRICORN_RENDER_OUT") {
            std::fs::create_dir_all(&dir).unwrap();
            let file = std::fs::File::create(Path::new(&dir).join(format!("bedroom-{gender}.png")))
                .unwrap();
            let mut enc = png::Encoder::new(file, 256, 192);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.write_header()
                .unwrap()
                .write_image_data(pixels.as_flattened())
                .unwrap();
        }
        images.push(pixels.to_vec());
    }
    assert_ne!(
        images[0], images[1],
        "gender selects the actual overworld texture"
    );
}
