{
  description = "GNOME Quickshare — native Quick Share for GNOME";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs = { self, nixpkgs }: let
    systems = [ "x86_64-linux" "aarch64-linux" ];
    eachSystem = nixpkgs.lib.genAttrs systems;
    packageFor = pkgs: pkgs.rustPlatform.buildRustPackage {
      pname = "gnome-quickshare";
      version = "0.1.0";
      src = nixpkgs.lib.cleanSourceWith {
        src = self;
        filter = path: type: !(builtins.elem (baseNameOf path) [ "target" ".flatpak-builder" "__pycache__" ".git" ".agents" ".codex" ".aws" "docs" "extension" "scripts" "result" ".github" "README.md" "justfile" ".gitignore" "flake.nix" "flake.lock" ]);
      };
      cargoLock = { lockFile = builtins.toFile "gnome-quickshare-Cargo.lock" (builtins.readFile ./Cargo.lock); allowBuiltinFetchGit = true; };
      nativeBuildInputs = with pkgs; [ pkg-config wrapGAppsHook4 protobuf gettext appstream ];
      buildInputs = with pkgs; [ gtk4 libadwaita dbus openssl ];
      cargoTestFlags = [ "--workspace" "--no-default-features" ];
      postInstall = ''
        bash po/install.sh "$out"
        install -Dm644 LICENSE "$out/share/licenses/gnome-quickshare/LICENSE"
        install -Dm644 vendor/rqs_lib/LICENSE "$out/share/licenses/gnome-quickshare/rqs-lib-LICENSE"
        install -Dm644 vendor/rqs_lib/LICENSE-APACHE "$out/share/licenses/gnome-quickshare/protobuf-LICENSE-APACHE"
        install -Dm644 vendor/rqs_lib/LICENSE-CHROMIUM-BSD "$out/share/licenses/gnome-quickshare/protobuf-LICENSE-CHROMIUM-BSD"
        cp -r data/icons $out/share/
        install -Dm644 data/io.github.oddship.GnomeQuickshare.service $out/share/dbus-1/services/io.github.oddship.GnomeQuickshare.service
        substituteInPlace $out/share/dbus-1/services/io.github.oddship.GnomeQuickshare.service --replace-fail "Exec=gnome-quickshare" "Exec=$out/bin/gnome-quickshare"
        install -Dm644 data/gnome-quickshare-nautilus.py $out/share/nautilus-python/extensions/gnome-quickshare.py
      '';
      meta = { description = "Quick Share for the GNOME desktop"; license = pkgs.lib.licenses.gpl3Plus; mainProgram = "gnome-quickshare"; platforms = systems; };
    };
  in {
    packages = eachSystem (system: let pkgs = import nixpkgs { inherit system; }; in {
      default = packageFor pkgs;
      gnome-extension = pkgs.stdenvNoCC.mkDerivation {
        pname = "gnome-quickshare-indicator"; version = "0.1.0"; src = ./extension; dontBuild = true;
        installPhase = "mkdir -p $out/share/gnome-shell/extensions/gnome-quickshare@oddship.github.io; cp extension.js metadata.json $out/share/gnome-shell/extensions/gnome-quickshare@oddship.github.io/";
      };
    });
    devShells = eachSystem (system: let pkgs = import nixpkgs { inherit system; }; in {
      default = pkgs.mkShell {
        packages = with pkgs; [ cargo rustc rustfmt clippy pkg-config gtk4 libadwaita dbus openssl protobuf xorg-server imagemagick adwaita-icon-theme desktop-file-utils appstream gettext just xdotool xprop python3 ripgrep gjs at-spi2-core ];
        GI_TYPELIB_PATH = pkgs.lib.makeSearchPath "lib/girepository-1.0" (map pkgs.lib.getLib [ pkgs.glib pkgs.at-spi2-core pkgs.gtk4 pkgs.libadwaita pkgs.gobject-introspection pkgs.graphene pkgs.pango pkgs.gdk-pixbuf pkgs.harfbuzz pkgs.cairo ]);
        shellHook = ''export CARGO_BUILD_JOBS=2'';
      };
      flatpak = pkgs.mkShell {
        packages = with pkgs; [ flatpak-builder appstream desktop-file-utils git curl just (python3.withPackages (p: [ p.aiohttp p.tomlkit ])) ];
      };
    });
    nixosModules.default = { config, lib, pkgs, ... }: let cfg = config.programs.gnome-quickshare; in {
      options.programs.gnome-quickshare = {
        enable = lib.mkEnableOption "GNOME Quickshare";
        openFirewall = lib.mkEnableOption "Quick Share mDNS and TCP port 53317 (set this port in Preferences)";
      };
      config = lib.mkIf cfg.enable {
        environment.systemPackages = [ (packageFor pkgs) ];
        networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ 53317 ];
        networking.firewall.allowedUDPPorts = lib.mkIf cfg.openFirewall [ 5353 ];
      };
    };
  };
}
