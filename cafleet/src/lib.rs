// Defined before every `mod` declaration: `macro_rules!` is visible only to modules declared after it.
#[cfg(test)]
macro_rules! test_path {
    ($name:ident) => {{
        let _: fn() = $name;
        concat!(module_path!(), "::", stringify!($name))
            .strip_prefix("cafleet::")
            .expect("test_path! names a test inside the cafleet crate")
    }};
}

#[cfg(test)]
macro_rules! assert_no_debug_or_serialize {
    ($($type:ty),+ $(,)?) => {{
        trait AmbiguousIfImplemented<Marker> {
            fn check() {}
        }
        struct DebugImplemented;
        struct SerializeImplemented;
        impl<T: ?Sized> AmbiguousIfImplemented<()> for T {}
        impl<T: ?Sized + std::fmt::Debug> AmbiguousIfImplemented<DebugImplemented> for T {}
        impl<T: ?Sized + serde::Serialize> AmbiguousIfImplemented<SerializeImplemented> for T {}
        // A forbidden trait implementation makes the inferred marker ambiguous.
        $(let _ = <$type as AmbiguousIfImplemented<_>>::check;)+
    }};
}

pub mod assets;
pub mod broker;
pub(crate) mod capture;
pub mod cli;
pub mod coding_agent;
pub mod config;
pub mod config_dir;
pub mod db;
pub(crate) mod deletion;
pub mod delivery;
pub(crate) mod diagnosis;
pub mod embedded;
pub mod error;
pub(crate) mod listen;
pub mod multiplexer;
pub mod output;
pub mod presentation;
pub(crate) mod receive;
pub mod runtime;
pub(crate) mod signals;
pub mod spawn_prompt;
pub mod time;

pub mod input;
