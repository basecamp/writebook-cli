# syntax=docker/dockerfile:1
# check=error=true

# The writebook CLI, mainly for serving a Writebook to MCP clients:
#
#   docker run -p 8080:8080 ghcr.io/basecamp/writebook-cli \
#     mcp --instance https://books.example.com --serve 0.0.0.0:8080

ARG RUST_VERSION=1.99
FROM docker.io/library/rust:${RUST_VERSION}-slim-trixie AS build

# OpenSSL and D-Bus are built from source for the keyring
RUN apt-get update -qq && \
    apt-get install --no-install-recommends -y make perl pkg-config && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked && \
    cp target/release/writebook /usr/local/bin/writebook


FROM docker.io/library/debian:trixie-slim

ARG OCI_SOURCE=https://github.com/basecamp/writebook-cli
ARG OCI_DESCRIPTION="Writebook CLI, MCP and agent skills"
LABEL org.opencontainers.image.source=${OCI_SOURCE} \
      org.opencontainers.image.description=${OCI_DESCRIPTION}

RUN apt-get update -qq && \
    apt-get install --no-install-recommends -y ca-certificates && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --create-home --uid 1000 writebook

COPY --from=build /usr/local/bin/writebook /usr/local/bin/writebook

USER writebook
ENV WRITEBOOK_NO_KEYRING=1
EXPOSE 8080
ENTRYPOINT ["writebook"]
CMD ["--help"]
