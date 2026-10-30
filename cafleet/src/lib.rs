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

pub(crate) mod assets;
pub(crate) mod broker;
pub(crate) mod capture;
pub mod cli;
pub(crate) mod coding_agent;
pub(crate) mod config;
pub(crate) mod config_dir;
pub(crate) mod db;
pub(crate) mod deletion;
pub(crate) mod delivery;
pub(crate) mod diagnosis;
pub(crate) mod digest;
pub(crate) mod embedded;
pub mod error;
pub(crate) mod input;
pub(crate) mod listen;
pub(crate) mod multiplexer;
pub(crate) mod output;
pub(crate) mod presentation;
pub(crate) mod receive;
pub(crate) mod runtime;
pub(crate) mod signals;
pub(crate) mod spawn_prompt;
pub(crate) mod time;
