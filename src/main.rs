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
#[cfg(test)]
macro_rules! assert_not_impl {
    ($type:ty: $($bound:path),+ $(,)?) => {{
        $({
            trait AmbiguousIfImplemented<Marker> {
                fn check() {}
            }
            struct Implemented;
            impl<T: ?Sized> AmbiguousIfImplemented<()> for T {}
            impl<T: ?Sized + $bound> AmbiguousIfImplemented<Implemented> for T {}
            let _ = <$type as AmbiguousIfImplemented<_>>::check;
        })+
    }};
}

pub(crate) mod assets;
pub(crate) mod broker;
mod cli;
pub(crate) mod coding_agent;
pub(crate) mod config;
pub(crate) mod db;
pub(crate) mod delivery;
pub(crate) mod digest;
mod error;
pub(crate) mod multiplexer;
pub(crate) mod output;
pub(crate) mod pane_lock;
pub(crate) mod runtime;
pub(crate) mod time;

fn main() {
    if let Err(error) = cli::run() {
        eprintln!("Error: {}", error.message());
        std::process::exit(error.exit_code());
    }
}
