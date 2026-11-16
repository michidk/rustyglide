# Runs rustyglide's GPU tests without a GPU: lavapipe (software Vulkan) and
# llvmpipe (software OpenGL). Test binaries are built on the host and mounted.
FROM ubuntu:26.04@sha256:d66b366dac61b575bee2aef07e9c26e8070096fbe84a3d4d3eb0da069b5d76db
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends mesa-vulkan-drivers libvulkan1 libegl1 libgl1-mesa-dri \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /work
