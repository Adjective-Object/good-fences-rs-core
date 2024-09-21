FROM ubuntu:24.10

# Update default packages
RUN apt-get update

# Set locale before any packages are installed,
# since packages are influenced by the current locale on installation
#
# This is necessary to get zsh to work well.
#  See: https://github.com/ohmyzsh/ohmyzsh/wiki/FAQ#i-see-duplicate-typed-characters-after-i-complete-a-command
RUN apt-get install -y language-pack-en
RUN echo LC_CTYPE=\"en_US.UTF-8\" "\n" \
    LC_ALL=\"en_US.UTF-8\" "\n" \
    LANG=\"en_US.UTF-8\" "\n" > /etc/default/locale

# Get Ubuntu packages
RUN apt-get install -y \
    build-essential \
    gdb \
    pkg-config \
    curl git zsh \
    vim

# Update new packages
RUN apt-get update

ARG USERNAME=ubuntu
ARG HOME=/home/${USERNAME}
ARG USER_UID=1010
ARG USER_GID=$USER_UID

# Create the user (disabled: user is default 'ubuntu')
# RUN groupadd --gid $USER_GID $USERNAME \
#     && useradd --uid $USER_UID --gid $USER_GID -m $USERNAME

# [Optional] Add sudo support. Omit if you don't need to install software after connecting.
RUN apt-get update \
    && apt-get install -y sudo \
    && echo $USERNAME ALL=\(root\) NOPASSWD:ALL > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME

# Get Node
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | sudo bash - 
RUN apt install -y nodejs

# Preload yarn with corepack
RUN corepack enable
RUN corepack prepare yarn@stable --activate

# get native dependencies
RUN apt install -y libdw-dev

# Start acting as the container user to avoid permissions issues
USER ${USERNAME}
RUN sudo chown ${USERNAME}:${USERNAME} ${HOME}/.cache -R

# setup zsh for the user
RUN mkdir -p ${HOME}/.antigen
RUN curl -L https://git.io/antigen > ${HOME}/.antigen/antigen.zsh
COPY zshrc.zsh ${HOME}/.zshrc
RUN sudo chown ${USERNAME}:${USERNAME} ${HOME}/.zshrc
# run zsh to install antigen plugins for the first time
RUN zsh ${HOME}/.zshrc

# Setup cargo directories as symlinks into the /cache volume
RUN sudo mkdir -p \
    /cache/cargo-registry \
    /cache/cargo-too
RUN sudo chown ${USERNAME}:${USERNAME} /cache -R
RUN mkdir -p \
    /home/${USERNAME}/.cargo/registry \
    /home/${USERNAME}/.cargo/toolchains
RUN sudo ln -s /cache/cargo-registry /home/${USERNAME}/.cargo/registry
RUN sudo ln -s /cache/cargo-toolchains /home/${USERNAME}/.cargo/toolchains 

# Install rust
RUN curl https://sh.rustup.rs -sSf | bash -s -- -y
RUN sudo chown ${USERNAME}:${USERNAME} ${HOME}/.cargo -R
RUN echo 'source ${HOME}/.cargo/env' >> ${HOME}/.zprofile
RUN echo 'source ${HOME}/.cargo/env' >> ${HOME}/.profile
