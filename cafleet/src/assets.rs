use include_dir::{Dir, include_dir};

pub static SKILLS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../skills");

pub static PRESETS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../presets");
