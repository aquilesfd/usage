# usage — build and install helpers

default: build

build:
    cargo build

release:
    cargo build --release --features applet

applet:
    cargo run --features applet --bin usage-applet

# Install binaries, desktop entry, icon, and the 5-minute systemd timer.
install:
    ./packaging/install.sh

uninstall:
    ./packaging/uninstall.sh

capture:
    cargo run --release -- capture

history:
    cargo run --release -- history

fmt:
    cargo fmt --all

lint:
    cargo clippy --all-targets --features applet -- -D warnings
