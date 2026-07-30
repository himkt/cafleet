use include_dir::{Dir, include_dir};

#[allow(dead_code)]
pub static MIGRATIONS: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/migrations");
