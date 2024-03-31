FROM ubuntu:22.04

WORKDIR /home/

COPY . .

ARG USERNAME=javachaos
ARG USER_UID=1000
ARG USER_GID=$USER_UID

# Create the user
RUN groupadd --gid $USER_GID $USERNAME \
    && useradd --uid $USER_UID --gid $USER_GID -m $USERNAME \
    #
    # [Optional] Add sudo support. Omit if you don't need to install software after connecting.
    && apt-get update \
    && apt-get install -y sudo \
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
    && echo $USERNAME ALL=\(root\) NOPASSWD:ALL > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME

# [Optional] Set the default user. Omit if you want to keep the default as root.
USER $USERNAME
RUN bash ./setup.sh
ENV PATH="/home/${USERNAME}/.cargo/bin:$PATH"
