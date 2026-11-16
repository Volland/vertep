# The pinned build image of Vertep (M13 production-readiness, "Reproducible Toolchain"): everything
# scripts/build.sh needs to build @vertep/web, @vertep/dm-ecma and the Python wheel from a clean
# checkout, and nothing from a maintainer's machine. CI builds both artifacts in it
# (.github/workflows/ci.yml, job `reproducible-build`); VS Code and GitHub Codespaces open the
# repository in it through .devcontainer/devcontainer.json.
#
# The build context is scripts/, so the image sees only scripts/toolchain.env, which holds every
# version below except the base image (scripts/check-toolchain.sh checks the FROM line against it):
#
#   docker build -f .devcontainer/Dockerfile -t vertep-build scripts
#   scripts/build.sh --container          # the same, then the build inside it
#
# Base: the official Rust image at the pinned toolchain, by digest (Debian trixie, gcc for the
# native build, rustup, cargo).
FROM rust:1.95.0-slim-trixie@sha256:e14e87345b4d5964ddcc3491d27ee046a0f23820f340c3c1e24da6880141f7c0

ARG TARGETARCH
COPY toolchain.env /etc/vertep/toolchain.env
SHELL ["/bin/bash", "-euo", "pipefail", "-c"]

# clang and llvm-ar for wasm32 (sqlite-wasm-rs compiles SQLite's C sources), Python for maturin
# and the wheel's smoke test, git for cargo and npm.
RUN . /etc/vertep/toolchain.env \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl git xz-utils pkg-config \
      "clang-${LLVM_MAJOR}" "llvm-${LLVM_MAJOR}" \
      python3 python3-venv python3-pip \
 && rm -rf /var/lib/apt/lists/* \
 && ln -s "/usr/bin/clang-${LLVM_MAJOR}" /usr/local/bin/clang \
 && ln -s "/usr/bin/llvm-ar-${LLVM_MAJOR}" /usr/local/bin/llvm-ar

# The Rust components and targets rust-toolchain.toml names, installed now so that a build never
# writes to the toolchain; and cargo's and rustup's homes writable by any user, so the container
# can run as the host user and leave files the host owns.
RUN rustup component add rustfmt clippy \
 && rustup target add wasm32-unknown-unknown thumbv7em-none-eabihf \
 && chmod -R a+rwX "$CARGO_HOME" "$RUSTUP_HOME"

# Node.js, from the official tarball, checksum-pinned.
RUN . /etc/vertep/toolchain.env \
 && case "$TARGETARCH" in \
      amd64) arch=x64; sum="$NODE_SHA256_X64" ;; \
      arm64) arch=arm64; sum="$NODE_SHA256_ARM64" ;; \
      *) echo "unsupported architecture $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && file="node-v${NODE_VERSION}-linux-${arch}.tar.xz" \
 && curl -fsSLo "/tmp/$file" "https://nodejs.org/dist/v${NODE_VERSION}/$file" \
 && echo "$sum  /tmp/$file" | sha256sum -c - \
 && tar -xJf "/tmp/$file" -C /usr/local --strip-components=1 --exclude='*.md' --exclude=LICENSE \
 && rm "/tmp/$file" \
 && node --version

# The wasm-bindgen CLI, from the upstream release, checksum-pinned (it must equal the library).
RUN . /etc/vertep/toolchain.env \
 && case "$TARGETARCH" in \
      amd64) arch=x86_64; sum="$WASM_BINDGEN_SHA256_X86_64" ;; \
      arm64) arch=aarch64; sum="$WASM_BINDGEN_SHA256_AARCH64" ;; \
      *) echo "unsupported architecture $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && name="wasm-bindgen-${WASM_BINDGEN_VERSION}-${arch}-unknown-linux-musl" \
 && curl -fsSLo "/tmp/$name.tar.gz" "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${WASM_BINDGEN_VERSION}/$name.tar.gz" \
 && echo "$sum  /tmp/$name.tar.gz" | sha256sum -c - \
 && tar -xzf "/tmp/$name.tar.gz" -C /tmp \
 && install -m 0755 "/tmp/$name/wasm-bindgen" /usr/local/bin/wasm-bindgen \
 && rm -rf "/tmp/$name" "/tmp/$name.tar.gz" \
 && wasm-bindgen --version

# maturin, in its own environment, at the pinned version.
RUN . /etc/vertep/toolchain.env \
 && python3 -m venv /opt/maturin \
 && /opt/maturin/bin/pip install --no-cache-dir "maturin==${MATURIN_VERSION}" \
 && ln -s /opt/maturin/bin/maturin /usr/local/bin/maturin \
 && maturin --version

ENV CC_wasm32_unknown_unknown=clang \
    AR_wasm32_unknown_unknown=llvm-ar \
    npm_config_update_notifier=false
WORKDIR /work
