{
  description = "Apeiron: backups, sync, versions and archives in one engine";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
    in
    {
      packages = forAll (pkgs: rec {
        # The `apeiron` command and its short name `ape` (docs/GUIDE.md §2.4).
        apeiron = pkgs.rustPlatform.buildRustPackage {
          pname = "apeiron";
          inherit version;
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [
            "-p"
            "apeiron-cli-rs"
          ];
          nativeBuildInputs = [
            pkgs.cmake
            pkgs.perl
            pkgs.pkg-config
          ];
          buildInputs = [ pkgs.zstd ];
          # aws-lc is built from its own sources, as on every other system;
          # the hooks of nixpkgs ask for a system copy otherwise.
          preBuild = ''
            export AWS_LC_SYS_USE_SYSTEM=0
          '';
          # The tests need the wasm32-wasip2 target and a network; CI runs them.
          doCheck = false;
          meta = {
            description = "Backups, sync, versions and archives in one engine";
            homepage = "https://github.com/efe-clk/apeiron";
            license = with pkgs.lib.licenses; [
              mit
              asl20
            ];
            mainProgram = "apeiron";
          };
        };
        default = apeiron;
      });
    };
}
