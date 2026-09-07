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
          # Lower-case names, not the `xorg.*` set: nixpkgs deprecated that
          # alias and every entry printed a rename warning on every `nix`
          # invocation -- eleven lines in front of each command, which is the
          # kind of noise that trains a reader to skim past output that
          # sometimes matters.
          guiLibs = with pkgs; [
            libglvnd # libGL.so.1 -- the dispatch library, not the driver
            alsa-lib # else SDL logs an audio failure every frame
            libx11
            libxrandr
            libxinerama
            libxcursor
            libxi
            libxext
            libxfixes
            libxrender
            libxcb
            libxau
            libxdmcp
          ];

          # Recording the graphical host as VIDEO instead of screenshotting it.
          #
          # `ffmpeg-full`, not `ffmpeg`: the default build has no x11grab. The
          # ffmpeg already on this machine's PATH lists only kmsgrab, fbdev,
          # v4l2 and audio -- kmsgrab needs root and takes the whole physical
          # display, which is not what we want. x11grab (xcbgrab) takes a
          # `window_id`, so it records the Factorio window alone and keeps
          # following it, ignoring the rest of the desktop.
          #
          # x11 and not a wayland recorder even on a wayland session, because
          # `SDL_VIDEODRIVER = "x11"` above means Factorio is an Xwayland
          # client: it really is an X11 window with an X11 window id.
          #
          # xdotool finds that id (`search --class factorio`); xwininfo reads
          # its geometry, which the recorder needs when the window is resized.
          captureTools = with pkgs; [ ffmpeg-full xdotool xwininfo ];
        in {
          # Only native/system libraries live here; the language toolchains are
          # pinned in mise.toml (rust, node, pnpm).
          default = pkgs.mkShell {
            # `mold` is the linker; `.cargo/config.toml` points rustc at it
            # with `-C link-arg=-fuse-ld=mold` (gcc 15 here, which supports
            # the flag). Linux only -- mold does not link Mach-O.
            nativeBuildInputs = with pkgs; [ pkg-config sccache ]
              ++ lib.optionals stdenv.hostPlatform.isLinux
                   ([ patchelf file chromium mold ] ++ captureTools);

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

            # Compilation cache SHARED ACROSS WORKTREES, which is the whole
            # point: agents work in separate `.worktrees/` checkouts, each with
            # its own `target/`, so identical dependency crates were compiling
            # once per worktree. Measured 2026-09-06: 163 GB of build artefacts
            # across nine target directories, and a load average of 85 on a
            # 20-core box with five builds running.
            #
            # sccache and NOT a shared `CARGO_TARGET_DIR`, deliberately. One
            # target directory would make every worktree write the same
            # `target/debug/factorio-bot`, so an offline `plan` or `score-map`
            # would silently run whichever branch built last and attribute its
            # numbers to the wrong code. That is the same family as the stale
            # mod that voided two findings, and worse: the mod at least logs a
            # `Using mods directory` line, while a binary path logs nothing.
            # Per-worktree targets are wasteful, and the waste is what buys
            # attributability. sccache caches the *artefacts*, not the output
            # path, so each worktree keeps its own binary.
            # WHAT IT CAN AND CANNOT CACHE, measured rather than assumed.
            # sccache refuses incremental compilation, and both profiles in
            # `Cargo.toml` set `incremental = true`. A first build reported:
            #
            #   Non-cacheable reasons:  incremental 2, missing input 2
            #
            # So the workspace's OWN crates are never cached, and dependencies
            # -- which cargo builds non-incrementally -- are. That split is the
            # one we want and is why `incremental` stays on: incremental serves
            # the inner edit-rebuild loop inside one worktree, sccache serves
            # the cross-worktree cost of recompiling the same dependency tree
            # nine times. They cover different halves and do not compete.
            #
            # It also settles a change that was queued and is now NOT being
            # made: dropping `[profile.dev.package."*"] opt-level = "z"`. That
            # was proposed to cut the from-scratch dependency build, which is
            # exactly the cost sccache now pays once globally instead of once
            # per worktree -- while `opt-level = "z"` keeps dependency code
            # fast at test runtime, which matters when a test parses an 865 MB
            # world dump. Fixing the cost at its real cause removed the reason
            # to trade that away.
            RUSTC_WRAPPER = "${pkgs.sccache}/bin/sccache";

            # app/e2e/smoke.mjs drives this through playwright-core, which
            # never downloads a browser of its own.
            CHROMIUM_BIN =
              if stdenv.hostPlatform.isLinux then "${pkgs.chromium}/bin/chromium" else "";
          };
        });
    };
}
