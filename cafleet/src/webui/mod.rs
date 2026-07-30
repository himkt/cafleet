use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "webui-dist"]
pub struct WebuiDist;
