FROM        alpine:3.20.3@sha256:beefdbd8a1da6d2915566fde36db9db0b524eb737fc57cd1367effd16dc0d06d AS builder

# renovate: datasource=repology depName=alpine_3_20/cargo versioning=loose
ARG         CARGO_VERSION="1.78.0-r0"
# renovate: datasource=repology depName=alpine_3_20/fuse3 versioning=loose
ARG         FUSE3_VERSION="3.16.2-r0"

ARG         TARGETPLATFORM

RUN         apk add --no-cache \
              fuse3-dev=${FUSE3_VERSION} \
              cargo=${CARGO_VERSION}

WORKDIR     /build
COPY        . .

ARG         CARGO_TERM_COLOR="always"
ARG         CARGO_TARGET_DIR="/root/.cargo/target"
RUN         --mount=type=cache,sharing=locked,target=/root/.cargo,id=home-cargo-$TARGETPLATFORM \
            cargo build --release && \
            cp /root/.cargo/target/release/zipfs .


FROM        alpine:3.20.3@sha256:beefdbd8a1da6d2915566fde36db9db0b524eb737fc57cd1367effd16dc0d06d

# renovate: datasource=repology depName=alpine_3_20/gcc versioning=loose
ARG         GCC_VERSION="13.2.1_git20240309-r0"
# renovate: datasource=repology depName=alpine_3_20/fuse3 versioning=loose
ARG         FUSE3_VERSION="3.16.2-r0"

WORKDIR     /app

RUN         apk add --no-cache \
              fuse3=${FUSE3_VERSION} \
              libgcc=${GCC_VERSION} \
            && \
            sed -i 's/#user_allow_other/user_allow_other/g' /etc/fuse.conf

COPY        --from=builder --chown=nobody:nogroup /build/zipfs /app/

USER        nobody
STOPSIGNAL  SIGINT

ENTRYPOINT  [ "./zipfs" ]
