# The Linux build environment of scripts/check-other-os (the native
# libraries the Linux check needed in CI), for checking and testing the Linux
# build from a Mac.
FROM rust:1-trixie
RUN apt-get update && apt-get install -y --no-install-recommends \
      clang cmake pkg-config python3 libasound2-dev libssl-dev libgtk-3-dev \
      libxkbcommon-x11-dev libx11-xcb-dev libfontconfig1-dev libfreetype-dev \
      libxcursor-dev libxi-dev libxrandr-dev libopenblas-dev libvulkan-dev \
      glslc libshaderc-dev libdbus-1-dev xvfb xauth jq dbus-x11 \
    && rm -rf /var/lib/apt/lists/*
RUN rustup component add rustfmt clippy \
    && cargo install --locked cargo-nextest
ENV CC=clang CXX=clang++ BLA_VENDOR=OpenBLAS OPENSSL_NO_VENDOR=1 \
    CARGO_PROFILE_DEV_DEBUG=0 CARGO_TARGET_DIR=/target
WORKDIR /hex
