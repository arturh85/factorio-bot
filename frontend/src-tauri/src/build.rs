use config_struct::StructOptions;
use config_struct::{create_struct, SerdeSupport};

fn main() {
    let options = StructOptions {
        struct_name: "AppSettings".into(),
        const_name: Some("APP_SETTINGS_DEFAULT".into()),
        serde_support: SerdeSupport::Yes,
        use_serde_derive_crate: false,
        generate_load_fns: false,
        ..StructOptions::default()
    };
    create_struct("AppSettings.toml", "src/app_settings/types.rs", &options)
        .expect("failed to generate app settings struct");
    tauri_build::build();
}
