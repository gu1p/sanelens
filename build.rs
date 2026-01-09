use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const UI_ASSETS: [&str; 3] = ["index.html", "app.js", "styles.css"];

fn main() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    println!("cargo:rerun-if-env-changed=COMPOSE_UI_DIST_DIR");

    let dist_dir = match env::var("COMPOSE_UI_DIST_DIR") {
        Ok(value) => resolve_path(&manifest_dir, value),
        Err(_) => panic!(
            "COMPOSE_UI_DIST_DIR is not set. Build the UI and set COMPOSE_UI_DIST_DIR to the \
dist directory."
        ),
    };

    for asset in UI_ASSETS {
        let path = dist_dir.join(asset);
        println!("cargo:rerun-if-changed={}", path.display());
        if !path.is_file() {
            panic!("missing UI asset: {}", path.display());
        }
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let out_ui_dir = out_dir.join("compose-ui");
    fs::create_dir_all(&out_ui_dir).expect("create ui out dir");
    for asset in UI_ASSETS {
        let src = dist_dir.join(asset);
        let dest = out_ui_dir.join(asset);
        fs::copy(&src, &dest).expect("copy ui asset");
    }

    println!(
        "cargo:rustc-env=COMPOSE_UI_INDEX_HTML={}",
        out_ui_dir.join("index.html").display()
    );
    println!(
        "cargo:rustc-env=COMPOSE_UI_APP_JS={}",
        out_ui_dir.join("app.js").display()
    );
    println!(
        "cargo:rustc-env=COMPOSE_UI_STYLES_CSS={}",
        out_ui_dir.join("styles.css").display()
    );
}

fn resolve_path(manifest_dir: &Path, value: String) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        manifest_dir.join(path)
    }
}
