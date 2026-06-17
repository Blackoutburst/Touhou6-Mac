//! WASM entry point. The browser uploads the player's own TH04 data archive
//! (`東方幻想.郷`); the bytes never leave their machine. We pick the uploaded
//! file that parses as a TH04 PAR archive, set up the stage, and run it on a
//! canvas. Nothing is bundled, fetched, or served.

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

use th04_formats::par::Archive;
use th06_engine::{Engine, SCREEN_H, SCREEN_W};

use crate::{make_update, setup};

/// Invoked from JS once the player selects their game folder. `files` maps each
/// uploaded file's basename to a `Uint8Array`.
#[wasm_bindgen]
pub async fn start_game(files: js_sys::Object) {
    console_error_panic_hook::set_once();

    // The main data archive is whichever uploaded file parses as a PAR archive
    // (pick the largest, i.e. 東方幻想.郷 over 幻想郷ED.DAT).
    let mut best: Option<Vec<u8>> = None;
    for entry in js_sys::Object::entries(&files).iter() {
        let pair: js_sys::Array = entry.into();
        let bytes = js_sys::Uint8Array::new(&pair.get(1)).to_vec();
        let bigger = best.as_ref().map_or(true, |b: &Vec<u8>| bytes.len() > b.len());
        if bigger && Archive::parse(bytes.clone()).is_ok() {
            best = Some(bytes);
        }
    }
    let arc = Archive::parse(best.expect("no TH04 archive (東方幻想.郷) found")).expect("parse archive");

    // WebGL needs the canvas to exist before the adapter is requested.
    let document = web_sys::window().expect("window").document().expect("document");
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .expect("create canvas")
        .dyn_into()
        .expect("canvas element");
    canvas.set_width(SCREEN_W);
    canvas.set_height(SCREEN_H);
    canvas.set_id("th04-canvas");
    canvas.set_tab_index(0); // focusable for keyboard input
    document.body().expect("body").append_child(&canvas).expect("append canvas");
    let _ = canvas.focus();

    let (engine, surface) = Engine::new_web(canvas.clone()).await;
    let (textures, dd, sim) = setup(&engine, &arc, "ST00.STD", 2);
    engine.run_game_web(canvas, surface, "Touhou 4 ~ Lotus Land Story", textures, make_update(sim, dd));
}
