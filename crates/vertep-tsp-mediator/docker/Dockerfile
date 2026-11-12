# vt-mediator: a TSP mailbox mediator.
#
# Build from the repository root:
#   docker build -f crates/vertep-tsp-mediator/docker/Dockerfile -t vertep/vt-mediator .
#
# Configuration comes from /etc/vt-mediator/mediator.toml (optional) and VT_MEDIATOR_*
# variables; keys, identity and mailboxes live in /var/lib/vt-mediator (mount a volume).

FROM rust:1.95.0-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY examples examples
RUN cargo build --release --locked -p vertep-tsp-mediator --bin vt-mediator

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --home /var/lib/vt-mediator vt \
 && mkdir -p /var/lib/vt-mediator /etc/vt-mediator \
 && chown vt /var/lib/vt-mediator
COPY --from=build /src/target/release/vt-mediator /usr/local/bin/vt-mediator
USER vt
ENV VT_MEDIATOR_LISTEN=0.0.0.0:8700 \
    VT_MEDIATOR_DATA_DIR=/var/lib/vt-mediator
EXPOSE 8700
VOLUME ["/var/lib/vt-mediator"]
HEALTHCHECK --interval=5s --timeout=3s --retries=12 \
  CMD curl -fsS http://127.0.0.1:8700/health || exit 1
ENTRYPOINT ["vt-mediator"]
CMD ["serve"]
