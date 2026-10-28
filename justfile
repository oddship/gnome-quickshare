set shell := ["sh", "-cu"]

run *args:
    cargo run -- {{args}}

build:
    cargo build --locked

check:
    cargo fmt --all --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --locked --workspace --no-default-features

package:
    nix build --cores 2 --max-jobs 1

# Use `nix develop .#flatpak` for the optional Flatpak tools.
flatpak:
    scripts/flatpak/build.sh

flatpak-sources:
    scripts/flatpak/update-sources.sh

# Extract messages and merge the configured translation catalogs.
i18n-update:
    scripts/i18n/update.sh

i18n-build:
    bash po/install.sh /tmp/gnome-quickshare-translated
