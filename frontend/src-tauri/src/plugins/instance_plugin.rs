use tauri::plugin::Plugin;

pub(crate) struct InstancePlugin {
  // plugin state, configuration fields
}

impl InstancePlugin {
  // you can add configuration fields here,
  // see https://doc.rust-lang.org/1.0.0/style/ownership/builders.html
  pub fn new() -> Self {
    Self {}
  }
}

impl Plugin for InstancePlugin {
  /// The JS script to evaluate on init.
  /// Useful when your plugin is accessible through `window`
  /// or needs to perform a JS task on app initialization
  /// e.g. "window.localStorage = { ... the plugin interface }"
  fn init_script(&self) -> Option<String> {
    Some(
      "
console.log('hello rust');
        "
      .into(),
    )
  }

  //Callback invoked when the webview is created.
  // fn created(&self, _webview: &mut Webview) {}

  // Callback invoked when the webview is ready.
  // fn ready(&self, _webview: &mut Webview) {}

  // fn extend_api(&self, webview: &mut Webview, payload: &str) -> Result<bool, String> {
  //     // extend the API here, following the Command guide
  //     // if you're not going to use this, you can just remove it
  // }
}
