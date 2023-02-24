FROM ubuntu:22.10

# Update default packages
RUN apt-get update

# Get Ubuntu packages
RUN apt-get install -y \
    build-essential \
    curl git zsh \
    vim

# Update new packages
RUN apt-get update

ARG USERNAME=dev
ARG HOME=/home/${USERNAME}
ARG USER_UID=1000
ARG USER_GID=$USER_UID

# Create the user
RUN groupadd --gid $USER_GID $USERNAME \
    && useradd --uid $USER_UID --gid $USER_GID -m $USERNAME \
    #
    # [Optional] Add sudo support. Omit if you don't need to install software after connecting.
    && apt-get update \
    && apt-get install -y sudo \
    && echo $USERNAME ALL=\(root\) NOPASSWD:ALL > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME

# setup zsh for the user
RUN mkdir -p ${HOME}/.antigen
RUN curl https://raw.githubusercontent.com/zsh-users/antigen/master/bin/antigen.zsh > ${HOME}/.antigen/antigen.zsh
COPY zshrc.zsh ${HOME}/.zshrc

# Get Rust
RUN curl https://sh.rustup.rs -sSf | bash -s -- -y
RUN echo 'source ${HOME}/.cargo/env' >> ${HOME}/.zshrc

# Get Node
RUN curl -fsSL https://deb.nodesource.com/setup_19.x | bash -
RUN apt install -y nodejs

# Preload yarn with corepack
RUN corepack enable
RUN corepack prepare yarn@stable --activate

# recursively change ownership of the mounted files
# to the "dev" user
RUN chown ${USERNAME}:${USERNAME} ${HOME} -R