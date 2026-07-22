FROM mcr.microsoft.com/devcontainers/base:ubuntu-24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN dpkg --add-architecture i386 \
    && apt-get update \
    && apt-get install --yes --no-install-recommends \
        aria2 \
        at-spi2-core \
        build-essential \
        cabextract \
        ca-certificates \
        curl \
        dbus-x11 \
        desktop-file-utils \
        file \
        libayatana-appindicator3-dev \
        libarchive-tools \
        librsvg2-dev \
        libssl-dev \
        libwebkit2gtk-4.1-dev \
        libxdo-dev \
        p7zip-full \
        patchelf \
        pkg-config \
        python3-pyatspi \
        rpm \
        ripgrep \
        unzip \
        wine \
        wine32:i386 \
        wine64 \
        xauth \
        xvfb \
        xz-utils \
        zstd \
        zenity \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

RUN curl --fail --silent --show-error --location \
        https://raw.githubusercontent.com/Winetricks/winetricks/20260125/src/winetricks \
        --output /usr/local/bin/winetricks \
    && echo "431f82fc74000e6c864409f1d8fb495d696c03928808e3e8acffc45179312a7b  /usr/local/bin/winetricks" | sha256sum --check --strict \
    && chmod 0755 /usr/local/bin/winetricks

ENV DEBIAN_FRONTEND=
