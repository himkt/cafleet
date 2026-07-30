//! The build-time embedded trees: build.rs-generated `include_bytes!` tables
//! over `webui-dist/` and the repo-root `skills/` and `presets/` trees, plus
//! the SPA's closed extension → content-type map.

include!(concat!(env!("OUT_DIR"), "/embedded_data.rs"));

pub fn lookup(
    table: &'static [(&'static str, &'static [u8])],
    path: &str,
) -> Option<&'static [u8]> {
    table
        .iter()
        .find(|(entry, _)| *entry == path)
        .map(|(_, bytes)| *bytes)
}
