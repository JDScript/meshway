//! Apple FFI boundary. Handwritten code is safe Rust; only UniFFI-generated
//! ABI scaffolding may use unsafe. Control-plane contact is not tunnel health.

mod signin;
pub use signin::*;

uniffi::setup_scaffolding!();

#[derive(uniffi::Record)]
pub struct CoreInfo {
    pub version: String,
    pub configuration_schema: u32,
    pub maximum_routing_rules: u32,
}

#[uniffi::export]
pub fn core_info() -> CoreInfo {
    CoreInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        configuration_schema: meshway_protocol::config::CONFIGURATION_SCHEMA_VERSION,
        maximum_routing_rules: meshway_policy::MAX_RULES as u32,
    }
}

/// The shared policy validator; validation does not assign tags or grant access.
#[uniffi::export]
pub fn valid_tag_selector(value: String) -> bool {
    meshway_policy::valid_tag_selector(&value)
}
