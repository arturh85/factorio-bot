{
  description = "factorio-bot system dependencies (rust/node/yarn come from mise)";

  # channel tarball instead of github:NixOS/nixpkgs/nixos-unstable so that
  # locking/updating does not need an authenticated GitHub API request
  inputs.nixpkgs.url = "https://channels.nixos.org/nixos-unstable/nixexprs.tar.xz";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = f:
        nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in {
      devShells = forAllSystems (pkgs:
        let
          inherit (pkgs) lib stdenv;
          libs = with pkgs; [
            lua5_4 # mlua links against system lua 5.4
            openssl
          ] ++ lib.optionals stdenv.hostPlatform.isLinux (with pkgs; [
            webkitgtk_4_1 # also provides javascriptcoregtk-4.1
            libsoup_3
            gtk3
            gtksourceview3
            glib
            cairo
            pango
            gdk-pixbuf
            atk
            librsvg
            fuse # libfuse2, needed for AppImage bundling
          ]);
        in {
          # Only native/system libraries live here; the language toolchains are
          # pinned in mise.toml (rust, node, yarn).
          default = pkgs.mkShell {
            nativeBuildInputs = with pkgs; [ pkg-config ]
              ++ lib.optionals stdenv.hostPlatform.isLinux [ patchelf file ];

            buildInputs = libs;

            # build scripts and the built binaries load lua/webkit at runtime,
            # and nothing puts the nix store paths into their rpath
            LD_LIBRARY_PATH = lib.makeLibraryPath libs;
          };
        });
    };
}
