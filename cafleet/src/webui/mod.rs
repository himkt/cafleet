use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "webui-dist"]
#[allow(dead_code)]
pub struct WebuiDist;
