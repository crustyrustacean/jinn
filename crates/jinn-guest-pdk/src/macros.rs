//! Entry-point wiring for jinn plugins.
//!
//! A plugin crate makes two calls at crate root, after `wit_bindgen::generate!`:
//!
//! ```ignore
//! jinn_guest_pdk::plugin!();   // inject host/bag/manifest/plugin + prelude
//! jinn_guest_pdk::export_plugin!(Welcome);   // Guest impl + component export
//! ```
//!
//! `plugin!` emits the helper modules; `export_plugin!` generates the wit-bindgen
//! `Guest` impl (forwarding to the user's `Plugin` trait impl) and wires the
//! component export. Two calls because `generate!`'s `export!` macro is
//! `pub(crate)` and can't be reliably called from inside another `macro_rules!`.

/// Inject PDK helper modules + prelude into the plugin crate.
#[macro_export]
macro_rules! plugin {
    () => {
        /// Ergonomic host-import wrappers (PDK-injected).
        pub mod host {
            $crate::__jinn_pdk_host_body!();
        }

        /// State-bag helpers. Defaults to postcard; JSON via the PDK's `json-bag` feature.
        pub mod bag {
            $crate::__jinn_pdk_bag_body!();
        }

        /// Manifest + badge/style builders (PDK-injected).
        pub mod manifest {
            $crate::__jinn_pdk_manifest_body!();
        }

        /// User-facing `Plugin` trait with default no-op hooks (PDK-injected).
        pub mod plugin {
            $crate::__jinn_pdk_plugin_body!();
        }

        /// Convenience re-export. `use crate::prelude::*` in a plugin module.
        pub mod prelude {
            pub use crate::bag::{
                get_global_data, get_plugin_data, set_global_data, set_plugin_data,
            };
            pub use crate::exports::jinn::plugin::hooks::Guest;
            pub use crate::host;
            pub use crate::host::{CreateSessionOutcome, LlmOutcome};
            pub use crate::jinn::plugin::types::*;
            pub use crate::manifest::{
                BadgeDirective, Keybind, Manifest, Segment, Style, Tool, ToolParam, ToolScope,
            };
            pub use crate::plugin::Plugin;
            pub use $crate::serde::{Deserialize, Serialize};
        }
    };
}

/// Generate the `Guest` impl for `$ty` and wire the component export.
#[macro_export]
macro_rules! export_plugin {
    ($ty:ident) => {
        $crate::__jinn_pdk_guest_impl!($ty);
        export!($ty with_types_in crate);
    };
}
