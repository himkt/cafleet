use include_dir::{Dir, include_dir};

#[allow(dead_code)]
pub static SKILLS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../skills");

#[allow(dead_code)]
pub static PRESETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../presets");
