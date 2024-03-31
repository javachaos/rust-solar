FROM debian:latest

WORKDIR /home/

COPY . .

ARG USERNAME=javachaos
ARG USER_UID=1000
ARG USER_GID=$USER_UID

# Create the user
RUN groupadd --gid $USER_GID $USERNAME \
    && useradd --uid $USER_UID --gid $USER_GID -m $USERNAME -s /bin/bash \
    #
    # [Optional] Add sudo support. Omit if you don't need to install software after connecting.
    && apt-get update \
    && apt-get install -y sudo \
        ca-certificates \
        curl \
        git \
        gnupg2 \
        build-essential \
        openssl \
        pkg-config \
        libudev-dev \
        gcc-mingw-w64-base \
        gcc-mingw-w64-i686-win32-runtime \
        gcc-mingw-w64-i686 \
        vim \
    && echo $USERNAME ALL=\(root\) NOPASSWD:ALL > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME

# [Optional] Set the default user. Omit if you want to keep the default as root.
USER $USERNAME
RUN curl https://sh.rustup.rs --tlsv1.3 -sSf | sh -s -- -y \
    && . "$HOME/.cargo/env" \
    && rustup install nightly \
    && rustup component add rustfmt \
    && rustup component add rustfmt --toolchain nightly \
    && rustup component add clippy \
    && rustup component add clippy --toolchain nightly \
    && rustup target add i686-pc-windows-gnu \
    && git config --global core.editor 'vi'

ENV PATH="/home/${USERNAME}/.cargo/bin:$PATH"
