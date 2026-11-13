{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.hex;
in
{
  options.programs.hex = {
    enable = lib.mkEnableOption "Agent-Desk local voice dictation";
    package = lib.mkOption {
      type = lib.types.package;
      description = "Agent-Desk package to install.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = lib.elem pkgs.stdenv.hostPlatform.system [
          "x86_64-linux"
          "aarch64-linux"
        ];
        message = "The Agent-Desk Nix package supports x86_64-linux and aarch64-linux only.";
      }
    ];
    environment.systemPackages = [ cfg.package ];
  };
}
