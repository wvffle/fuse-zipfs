FROM        alpine:3.20.1@sha256:b89d9c93e9ed3597455c90a0b88a8bbb5cb7188438f70953fede212a0c4394e0 AS builder

# renovate: datasource=repology depName=alpine_3_20/cargo versioning=loose
ARG         CARGO_VERSION="1.78.0-r0"
# renovate: datasource=repology depName=alpine_3_20/fuse3 versioning=loose
ARG         FUSE3_VERSION="3.16.2-r0"

WORKDIR     /build
COPY        . .

ENV         CARGO_TERM_COLOR="always"
RUN         apk add --no-cache \
              fuse3-dev=${FUSE3_VERSION} \
              cargo=${CARGO_VERSION} \
            && \
            cargo build --release && \
            rm -rf /root/.cache /root/.cargo


FROM        alpine:3.20.1@sha256:b89d9c93e9ed3597455c90a0b88a8bbb5cb7188438f70953fede212a0c4394e0

# renovate: datasource=repology depName=alpine_3_20/gcc versioning=loose
ARG         GCC_VERSION="13.2.1_git20240309-r0"
# renovate: datasource=repology depName=alpine_3_20/fuse3 versioning=loose
ARG         FUSE3_VERSION="3.16.2-r0"

WORKDIR     /app
COPY        --from=builder --chown=nobody:nogroup /build/target/release/zipfs /app/

RUN         apk add --no-cache \
              fuse3=${FUSE3_VERSION} \
              libgcc=${GCC_VERSION} \
            && \
            sed -i 's/#user_allow_other/user_allow_other/g' /etc/fuse.conf

USER        nobody
STOPSIGNAL  SIGINT

ENTRYPOINT  [ "./zipfs" ]
