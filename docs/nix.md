# Nix

The flake packages Agent-Desk for Linux only, `x86_64-linux` and `aarch64-linux`;
there is no macOS package. Both targets build from source; `aarch64-linux`
evaluates but has not been built or run on real ARM64 hardware yet. It
supports the Linux X11 beta and native Wayland on compatible wlroots
compositors. Wayland requires layer-shell for the recording overlay, the
clipboard protocol used by `wl-copy`, and the virtual-keyboard protocol
used by `wtype`. Global shortcuts come from the desktop's GlobalShortcuts
portal, or from your own compositor keybindings running the `agent-desk` commands.
This is not a claim of support for every Wayland desktop; GNOME and KDE are
not covered by this contract.

## Run Or Install

On macOS, install the [DMG or Homebrew cask](../README.md#downloads) instead;
the flake has no `aarch64-darwin` package.

From a checkout, with Nix flakes enabled:

```sh
nix build
nix run . -- model install
nix run . -- app
```

Or add `hex.url = "github:anomalyco/hex"` to your configuration's flake inputs
and install `hex.packages.${system}.default` (`x86_64-linux` or
`aarch64-linux`). The package supplies the
`agent-desk` command and desktop launcher, native libraries (including PipeWire and
libva for Annotate), and the `curl`, `wl-copy`, `wtype`, and `ffmpeg`/`ffprobe`
runtime dependencies. Annotate's hardware encoder uses whatever VA-API driver
your system provides (NixOS: `hardware.graphics`, through `/run/opengl-driver`);
without one it falls back to ffmpeg's libx264. Your system still needs a
working audio session and a Vulkan driver for Settings, even when inference
falls back to the CPU. On non-NixOS hosts, Nix GUI
applications may require the host's usual Nix graphics integration.

On NixOS, Agent-Desk built from its own pinned nixpkgs can use an older glibc than
your system's graphics drivers need. Agent-Desk then finds no suitable Vulkan
adapter and its window does not open. Make Agent-Desk's nixpkgs follow your
system's, so both use the same glibc:

```nix
{
  inputs.hex = {
    url = "github:anomalyco/hex";
    # Use the name of your own nixpkgs input here.
    inputs.nixpkgs.follows = "nixpkgs";
  };
}
```

Models are downloaded into your user data directory, not the Nix store.
Run `agent-desk model install` once, then launch Agent-Desk as your normal desktop user.
Do not run Agent-Desk as root.

The Nix store is not Agent-Desk's user-local managed installation layout. In-app
updates do not replace a Nix installation. Update the flake input and
rebuild or switch your Nix/Home Manager configuration instead. Remove any
old source-install XDG autostart entry before enabling a second autostart
mechanism; do not run two Agent-Desk listeners.

## NixOS

Import `hex.nixosModules.hex` and enable the package:

```nix
{
  imports = [ hex.nixosModules.hex ];
  programs.hex.enable = true;
}
```

This installs Agent-Desk system-wide. It does not start a root service or enable
autostart for every account. Use the
Home Manager module below for per-user autostart. Both modules accept an
explicit `programs.hex.package` override. The flake also exports
`hex.overlays.default` for configurations that prefer `pkgs.hex`.

## Home Manager

```nix
{
  imports = [ hex.homeManagerModules.hex ];
  programs.hex = {
    enable = true;
    autostart = true;
    # Use your compositor's session target when appropriate:
    # systemdTarget = "sway-session.target";
  };
}
```

The module always installs the systemd **user** service. Autostart defaults to
`false`; when enabled, the unit is wanted by `graphical-session.target` by default.
It waits for a Wayland socket or an X11 window manager before launching `agent-desk service`,
which reports readiness to systemd (`Type=notify`) once its socket answers.
Agent-Desk is one process: `agent-desk` opens its window, starting the unit when it is not
running, and `agent-desk start` starts it without the window. Closing the window never
stops Agent-Desk; a tray icon shows when the desktop has a tray host. The unit stops with the configured target
and retries failed starts without a tight restart loop; a crash restart opens no
window. `agent-desk quit` (or **Quit Agent-Desk** in the window) leaves it stopped until the
next session or the next launch.

Your session manager must import its current `DISPLAY`, `XAUTHORITY`,
`WAYLAND_DISPLAY`, and `XDG_RUNTIME_DIR` values as applicable into the systemd
user environment **before** starting the target, and stop the target at
logout. Use your compositor's systemd integration, such as Home Manager's
Sway integration, rather than starting the target from a shell login.
The module does not create a graphical session or grant permission to use
one. A bare `startx`/i3 session without systemd session integration will not
start this service automatically.

`agent-desk start` records the current display/session environment in an owner-only
file read by the unit; it clears stale variables when switching between X11 and
Wayland. There is one Agent-Desk per user. Launching it from a different desktop
explicitly restarts the unit for that session. The unit names Agent-Desk's default
data folder, so Agent-Desk still treats it as its own supervised service: a stale
`XAUTHORITY` from an earlier login (GNOME's XWayland cookie) is replaced with
the user manager's current one, keeping the HUD working after a re-login (#1140).

Home Manager's normal service-switch policy applies: newly enabled services
can start during a switch if the session target is already active, and
package changes can restart a running service. Do not switch during an
active recording. Inspect failures with:

```sh
systemctl --user status agent-desk.service
journalctl --user -u agent-desk.service
```

### Declarative Settings

```nix
{
  programs.hex = {
    enable = true;
    shortcut = "control+option+space"; # dictation_hotkey
    model = "parakeet_v3";
    language = "de";
    installModel = true;
    settings = {
      shortcut_mode = "toggle";
      sound_effect_volume = 0.4;
      paste_with_shift = true;
      "meetings.enabled" = true;
    };
  };
}
```

`settings` takes the dotted keys `agent-desk settings list` shows, with JSON values:
booleans, numbers, text such as shortcuts and choices, `null` to clear an
optional setting, and attribute sets or lists for its `json` settings. The
`shortcut`, `model`, and `language` options are shorthands for
`dictation_hotkey`, `transcription.model`, and `transcription.language`.

Every activation merges them into Agent-Desk with `agent-desk settings apply`, after
systemd units reload. Declared keys overwrite Agent-Desk's values; anything you
change in the window that isn't declared keeps its value until you declare
it. A running Agent-Desk saves and adopts the settings at once, so it never writes
older values back from memory; with Agent-Desk stopped they go straight into
`settings.json`. `settings.json` stays an ordinary writable file. An unknown
key or invalid value applies nothing and prints a warning, without failing
the switch.

Declaring a model only selects it; activation never downloads. With
`installModel = true`, the `hex-model` user service runs `agent-desk model install`
for the selected model at login, once the system's `network-online.target`
is reached (or after two minutes), and retries a failed download. A switch
does not wait for it.

## Wayland Shortcuts

Agent-Desk reads no input devices, so neither module adds users to the `input`
group, installs udev rules, or grants capabilities. With a GlobalShortcuts
portal, the desktop grants the shortcuts Settings records; the portal
identifies Agent-Desk by the package's `fheuer.agentdesk.desktop` entry, so keep the package's
`share/applications` on `XDG_DATA_DIRS` (NixOS and Home Manager profiles do).
Without the portal, bind the commands in your compositor, for example in Sway:

```sh
bindsym F9 exec agent-desk dictation start
bindsym --release F9 exec agent-desk dictation stop
```

`agent-desk dictation toggle` and `agent-desk paste-last`
are also available; see the
[Linux guide](linux.md#shortcuts). The paste helper is `wtype`; no privileged
input-injection fallback is installed.

## Development

```sh
nix develop
cargo build --locked
cargo test --locked
cargo run --locked -- app
```

The package and shell share the native build environment and runtime
library/plugin paths. OpenBLAS link ordering is handled by `build.rs`, not
by Nix-only `RUSTFLAGS`. The shell also supplies the paste and download
helpers, so it does not rely on the installed app's wrapper. Desktop commands
still require a real graphical/audio session and the permissions described above.

`nix flake check` evaluates the module and shell-environment regressions and
builds the package with its normal display-free Rust tests. Tests explicitly
marked ignored because they require a desktop or installed model remain
ignored; the Nix build does not download models or exercise live input.

The initial packaging design builds on [PR #29](https://github.com/anomalyco/hex/pull/29).
Declarative settings follow
[@SymphonySimper's request in #588](https://github.com/anomalyco/hex/issues/588).
