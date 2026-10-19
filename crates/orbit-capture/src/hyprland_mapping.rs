//! Client bindings for `hyprland-toplevel-mapping-v1`, generated from the
//! vendored XML in `protocols/`.

// The scanner's generated interface tables use unsafe statics.
#![allow(unsafe_code)]
#![allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    unused_imports,
    missing_docs,
    clippy::all
)]

pub mod client {
    use wayland_client;
    use wayland_client::protocol::*;
    use wayland_protocols::ext::foreign_toplevel_list::v1::client::*;
    use wayland_protocols_wlr::foreign_toplevel::v1::client::*;

    pub mod __interfaces {
        use wayland_client::protocol::__interfaces::*;
        use wayland_protocols::ext::foreign_toplevel_list::v1::client::__interfaces::*;
        use wayland_protocols_wlr::foreign_toplevel::v1::client::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/hyprland-toplevel-mapping-v1.xml");
    }
    use self::__interfaces::*;

    wayland_scanner::generate_client_code!("protocols/hyprland-toplevel-mapping-v1.xml");
}
