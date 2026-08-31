fn main() {
  luaify();
  #[cfg(windows)]
  {
    // set .exe file properties
    let mut res = winres::WindowsResource::new();
    res.set("ProductName", "Factorio-Bot");
    res.set("FileDescription", "Factorio-Bot");
    res.set("Version", env!("CARGO_PKG_VERSION"));
    res.set("LegalCopyright", "Copyright (C) 2022");
    res
      .compile()
      .expect("Failed to run the Windows resource compiler (rc.exe)");
  }
  println!("cargo:rerun-if-changed=../../crates/scripting_lua/src/");
}

fn luaify() {
  #[cfg(feature = "lua")]
  {
    use factorio_bot_scripting_lua::lua_docs::write_lua_docs;
    let path = std::path::Path::new(&format!(
      "{}/../../docs/lua/src/",
      env!("CARGO_MANIFEST_DIR")
    ))
    .to_path_buf();
    write_lua_docs(path).expect("Failed to write lua docs");
  }
}
