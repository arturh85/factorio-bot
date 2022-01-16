use config_struct::StructOptions;
use config_struct::{create_struct, SerdeSupport};

const RUST_SETTINGS_PATH: &str = "src/settings/types.rs";

fn main() {
    let options = StructOptions {
        struct_name: "AppSettings".into(),
        const_name: Some("APP_SETTINGS_DEFAULT".into()),
        serde_support: SerdeSupport::Yes,
        use_serde_derive_crate: false,
        generate_load_fns: false,
        derived_traits: vec![
            "Debug".into(),
            "Clone".into(),
            "typescript_definitions::TypeScriptify".into(),
        ],
        ..StructOptions::default()
    };
    create_struct("AppSettings.toml", RUST_SETTINGS_PATH, &options)
        .expect("failed to generate app settings struct");
}
