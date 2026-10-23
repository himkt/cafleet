pub mod assets;
pub mod broker;
pub(crate) mod capture;
pub mod cli;
pub mod coding_agent;
pub mod config;
pub mod config_dir;
pub mod db;
pub mod delivery;
pub(crate) mod diagnosis;
pub mod embedded;
pub mod error;
pub(crate) mod member_permissions;
pub mod multiplexer;
pub mod output;
pub mod pane_state;
pub mod presentation;
#[allow(
    dead_code,
    reason = "Receive behavior awaits reviewed timing contracts."
)]
pub(crate) mod receive;
pub mod runtime;
pub mod spawn_prompt;
pub mod time;

pub mod input;
