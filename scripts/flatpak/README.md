# Flatpak development package

The manifest targets GNOME 50 and its Freedesktop 25.08 Rust extension. It builds
from the current source directory with dependencies pinned by Cargo.lock and the
generated source manifest. Compilation and tests use Cargo's offline mode. The
app is exported on the `devel` branch; it is not a Flathub release.

## Build

Install flatpak-builder and appstreamcli, then install the build tools into your user installation.
Nix users can obtain flatpak-builder and the source-generator dependencies with
`nix develop path:.#flatpak`:

```sh
flatpak install --user flathub org.gnome.Sdk//50 org.freedesktop.Sdk.Extension.rust-stable//25.08
scripts/flatpak/build.sh /tmp/gnome-quickshare-flatpak
```

The script builds the application, runs the workspace tests with Bluetooth
hardware access disabled, exports an OSTree repository, and creates
`/tmp/gnome-quickshare-flatpak/gnome-quickshare-0.1.0.flatpak`.

Install that local development bundle with:

```sh
flatpak install --user /tmp/gnome-quickshare-flatpak/gnome-quickshare-0.1.0.flatpak
flatpak run io.github.oddship.GnomeQuickshare
```

The builder disables the optional rofiles-fuse cache layer, so building does not
require a FUSE device. Application sandboxing remains enabled.

The local bundle is unsigned. Review the source and permissions before installing
it. No public repository or marketplace listing is created by this build.

## Permissions and integration

The runtime permissions cover Wayland/X11 rendering, shared-memory rendering,
network discovery/transfers, the Downloads directory, BlueZ access, and an
optional status notifier host. There is no blanket home-directory or session-bus
access. File selection and background permission use desktop portals.

The default receive folder is Downloads/Quickshare. A custom folder must remain
accessible after app restarts; portal-provided access can be temporary. Grant a
specific permanent folder using Flatpak's filesystem override when needed.

The Python Files extension is a **host** Nautilus plugin. It is not installed
inside the application sandbox because that would not make Nautilus load it.
Install `nautilus-python` with your distribution, copy
`data/gnome-quickshare-nautilus.py` to your user Nautilus extensions directory,
and restart Files. The extension uses the exported desktop entry, whose file
arguments Flatpak forwards through the document portal.

## Update dependencies

After changing Cargo.lock, regenerate `cargo-sources.json`:

```sh
# Requires Python 3.11+, aiohttp, tomlkit, Git, curl, and sha256sum.
scripts/flatpak/update-sources.sh
```

The generator is pinned to flatpak-builder-tools commit
`9d86d336a39b9af38b86f114b6ce5820fe260f91`, with its own SHA-256 checked before
execution. The protobuf compiler archives use the architecture-specific versions
and checksums also used by Packet's GNOME 50 development manifest. Their full
layouts are retained (`strip-components: 0`) and build-only files are removed
from the finished application.

## Installed sandbox smoke check

With the app installed, build the native test peer (`cargo build
--no-default-features --features test-tools --bin quickshare-probe`) and run the
same isolated display/session-bus harness against the sandbox:

```sh
QUICKSHARE_FLATPAK_RUNTIME_DIR="$XDG_RUNTIME_DIR" \
QUICKSHARE_SMOKE_BACKGROUND=1 \
QUICKSHARE_SMOKE_PROBE="$PWD/target/debug/quickshare-probe" \
scripts/ui-smoke.sh "$PWD/scripts/flatpak/test-launcher.sh" /tmp/quickshare-flatpak-ui
```

Use the normal development shell for Xvfb, xdotool, xprop and ImageMagick. The
launcher grants only the harness's temporary directory in addition to the app's
manifest permissions, and forwards temporary XDG paths inside the sandbox.
`FLATPAK_USER_DIR` can select a separate temporary app installation. This check
exercises background activation, desktop actions, consent, file receipt and
byte comparison, and narrow layouts. Leave `QUICKSHARE_SMOKE_SETTINGS` disabled:
the private bus has no background portal. Real document/background portals,
Bluetooth hardware, notifications and physical-device compatibility require
separate tests in an actual desktop session.

For the opt-in live Wayland/accessibility/notification check, use
`scripts/desktop-smoke.sh` with this launcher as its binary, set
`QUICKSHARE_FLATPAK_LIVE=1`, and keep `QUICKSHARE_FLATPAK_RUNTIME_DIR` pointing at
the actual desktop session's `$XDG_RUNTIME_DIR` (normally `/run/user/1000`).
A private runtime directory prevents the host portal from validating Flatpak's
instance metadata. `FLATPAK_USER_DIR` may still point at a temporary installation.
The live mode retains
the document and accessibility portals and binds the actual Wayland socket; the
isolated Xvfb mode disables them. A generated test config and receive directory
are granted in both modes. Background/file permission interactions need separate
portal checks; opening Preferences alone does not verify a permission grant.

The isolated launcher forwards `ADW_DEBUG_HIGH_CONTRAST` and
`ADW_DEBUG_COLOR_SCHEME` when explicitly set, for theme verification without
changing desktop preferences. `QUICKSHARE_SMOKE_WIFI=1` exercises the manual form,
synthetic incoming credential consent/review and password-only clipboard check;
`QUICKSHARE_SMOKE_EMPTY=1` separately exercises zero-byte file receipt.

With the live launcher and background portal check, add
`QUICKSHARE_SMOKE_TRAY=1 QUICKSHARE_SMOKE_TRAY_ACTIONS=1` to exercise actual tray
Visible/Hide/Open/Quit actions. The tray service is checked against the test app's
bus PID before menu events; the background setting is restored to off afterwards.
