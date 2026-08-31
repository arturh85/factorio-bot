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
            xz # liblzma, loaded at runtime by the compiled binaries
          ];

          # What the GRAPHICAL Factorio client needs to open a window. Not
          # needed to build anything -- only to run `factorio` with a display,
          # which is what screenshot capture requires: `game.take_screenshot`
          # does nothing on a headless server.
          #
          # This was diagnosed three times as "no display" when the display was
          # fine. The real chain, each step found only by fixing the one before
          # it:
          #
          #   1. SDL defaults to the wayland backend on a wayland session and
          #      fails with "No available video device" -- a message that says
          #      nothing about the cause. `SDL_VIDEODRIVER=x11` reveals the
          #      next error instead of hiding it.
          #   2. then "x11 not available", because libXrandr and friends are
          #      absent -- the shell has no X11 client libraries at all.
          #   3. then "Failed loading libGL.so.1", which is NOT in
          #      /run/opengl-driver/lib: that path holds the Mesa *driver*,
          #      not the GL dispatch library. Both are required.
          #
          # With all three: "Initialised OpenGL: AMD Radeon 880M (radeonsi)"
          # and "Factorio initialised".
          guiLibs = with pkgs; [
            libglvnd # libGL.so.1 -- the dispatch library, not the driver
            alsa-lib # else SDL logs an audio failure every frame
            xorg.libX11
            xorg.libXrandr
            xorg.libXinerama
            xorg.libXcursor
            xorg.libXi
            xorg.libXext
            xorg.libXfixes
            xorg.libXrender
            xorg.libxcb
            xorg.libXau
            xorg.libXdmcp
          ];
        in {
          # Only native/system libraries live here; the language toolchains are
          # pinned in mise.toml (rust, node, pnpm).
          default = pkgs.mkShell {
            nativeBuildInputs = with pkgs; [ pkg-config ]
              ++ lib.optionals stdenv.hostPlatform.isLinux [ patchelf file chromium ];

            buildInputs = libs;

            # build scripts and the built binaries load lua at runtime, and
            # nothing puts the nix store paths into their rpath.
            #
            # `/run/opengl-driver/lib` goes FIRST on Linux: it holds the host's
            # Mesa driver, which must match the running kernel, so a store path
            # shadowing it gives a GL context that does not work. The gui libs
            # follow it; `libs` stays last so nothing here shadows lua.
            LD_LIBRARY_PATH = lib.makeLibraryPath libs
              + lib.optionalString stdenv.hostPlatform.isLinux
                  (":/run/opengl-driver/lib:" + lib.makeLibraryPath guiLibs);

            # SDL picks the wayland backend by default on a wayland session and
            # fails there; Xwayland works. Set here rather than documented,
            # because a documented workaround is one every session rediscovers.
            SDL_VIDEODRIVER = if stdenv.hostPlatform.isLinux then "x11" else "";

            # app/e2e/smoke.mjs drives this through playwright-core, which
            # never downloads a browser of its own.
            CHROMIUM_BIN =
              if stdenv.hostPlatform.isLinux then "${pkgs.chromium}/bin/chromium" else "";
          };
        });
    };
}
