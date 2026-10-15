{
  lib,
  craneLib,
  buildNpmPackage,
  importNpmLock,
  nodejs,
  pkg-config,
  wrapGAppsHook3,
  webkitgtk_4_1,
  gtk3,
  libayatana-appindicator,
  librsvg,
  libsoup_3,
  openssl,
  glib-networking,
  dbus,
}:

let
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;
  fs = lib.fileset;

  uiFiles = fs.difference ../ui (
    fs.unions [
      (fs.maybeMissing ../ui/node_modules)
      (fs.maybeMissing ../ui/dist)
    ]
  );
  uiSrc = fs.toSource {
    root = ../ui;
    fileset = uiFiles;
  };

  # The web UI (popover and settings), embedded into the daemon at compile time.
  ui = buildNpmPackage {
    pname = "aw-manual-ui";
    inherit version nodejs;
    src = uiSrc;
    # Read the manifests directly (no import from derivation, so
    # `nix flake check --no-build` can evaluate the package).
    npmDeps = importNpmLock {
      package = lib.importJSON ../ui/package.json;
      packageLock = lib.importJSON ../ui/package-lock.json;
    };
    npmConfigHook = importNpmLock.npmConfigHook;
    installPhase = ''
      runHook preInstall
      cp -r dist $out
      runHook postInstall
    '';
  };

  # Arguments shared by the dependency and the package build. They must match,
  # otherwise cargo considers the cached dependencies stale and rebuilds them.
  commonArgs = {
    pname = "aw-manual";
    inherit version;

    src = fs.toSource {
      root = ../.;
      fileset = fs.unions [
        ../Cargo.toml
        ../Cargo.lock
        ../crates
        uiFiles
      ];
    };

    strictDeps = true;

    cargoExtraArgs = "--locked";
    cargoBuildExtraArgs = "-p aw-manual-cli -p aw-manual-daemon";

    nativeBuildInputs = [
      pkg-config
      wrapGAppsHook3
    ];

    buildInputs = [
      webkitgtk_4_1
      gtk3
      libayatana-appindicator
      librsvg
      libsoup_3
      openssl
      glib-networking
      dbus
    ];

    # Only the daemon needs the GTK environment; the CLI stays a plain binary.
    dontWrapGApps = true;
  };

  # All Rust dependencies, built from the Cargo manifests and the lock file
  # only. Rebuilt when Cargo.toml or Cargo.lock change, not on code changes.
  cargoArtifacts = craneLib.buildDepsOnly (
    commonArgs
    // {
      # The package build runs no `cargo check`; skip it here as well.
      cargoCheckCommand = "true";
      cargoTestExtraArgs = "--no-run -p aw-manual-core";
    }
  );
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;

    cargoTestExtraArgs = "-p aw-manual-core";

    preBuild = ''
      rm -rf ui/dist
      cp -r ${ui} ui/dist
      chmod -R u+w ui/dist
    '';

    postInstall = ''
      install -Dm644 crates/daemon/icons/32x32.png $out/share/icons/hicolor/32x32/apps/aw-manual.png
      install -Dm644 crates/daemon/icons/128x128.png $out/share/icons/hicolor/128x128/apps/aw-manual.png
      install -Dm644 crates/daemon/icons/128x128@2x.png $out/share/icons/hicolor/256x256/apps/aw-manual.png
      install -Dm644 /dev/stdin $out/share/applications/io.github.arstulke.aw-manual.desktop <<DESKTOP
      [Desktop Entry]
      Type=Application
      Name=aw-manual
      Comment=Manual activity tracking for ActivityWatch
      Exec=aw-manual-daemon
      Icon=aw-manual
      Categories=Office;Utility;
      StartupWMClass=aw-manual-daemon
      Terminal=false
      DESKTOP
    '';

    postFixup = ''
      wrapGApp $out/bin/aw-manual-daemon \
        --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [ libayatana-appindicator ]}
    '';

    passthru = { inherit ui cargoArtifacts; };

    meta = {
      description = "Fast manual activity tracking for ActivityWatch (popover, tray, hotkeys, CLI)";
      homepage = "https://github.com/arstulke/aw-manual";
      license = lib.licenses.mpl20;
      mainProgram = "aw-manual";
      platforms = lib.platforms.linux;
    };
  }
)
