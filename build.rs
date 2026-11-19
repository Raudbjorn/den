//! Generates `$OUT_DIR/oxide_palette.rs` from the committed IRONMAW Oxide tokens.
//! See `assets/tokens/SOURCE.md`.

#[path = "build/palette_gen.rs"]
mod palette_gen;

use std::{env, error::Error, fs, path::PathBuf};

const TOKENS_PATH: &str = "assets/tokens/oxide.tokens.json";
const GENERATOR_PATH: &str = "build/palette_gen.rs";
const OUTPUT_FILE: &str = "oxide_palette.rs";

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo::rerun-if-changed={TOKENS_PATH}");
    println!("cargo::rerun-if-changed={GENERATOR_PATH}");

    let tokens =
        fs::read_to_string(TOKENS_PATH).map_err(|e| format!("reading {TOKENS_PATH}: {e}"))?;
    let source = palette_gen::generate(&tokens).map_err(|e| format!("{TOKENS_PATH}: {e}"))?;

    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let out_path = out_dir.join(OUTPUT_FILE);
    fs::write(&out_path, source).map_err(|e| format!("writing {}: {e}", out_path.display()))?;
    Ok(())
}
