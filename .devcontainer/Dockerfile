FROM ubuntu:24.10

# Update default packages
RUN apt-get update

# Get Ubuntu packages
RUN apt-get install -y \
    build-essential \
    pkg-config \
    curl git zsh \
    vim

# Update new packages
RUN apt-get update

ARG USERNAME=vscode
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

# setup zsh for the user
RUN mkdir -p ${HOME}/.antigen
RUN curl -L https://git.io/antigen > ${HOME}/.antigen/antigen.zsh
COPY zshrc.zsh ${HOME}/.zshrc
RUN sudo chown ${USERNAME} ${HOME}/.zshrc
# configure antigen
RUN zsh -c "source ${HOME}/.antigen/antigen.zsh; antigen use oh-my-zsh &&\
    antigen bundle git &&\
    antigen bundle heroku &&\
    antigen bundle pip &&\
    antigen bundle lein &&\
    antigen bundle command-not-found &&\
    antigen bundle zsh-users/zsh-syntax-highlighting &&\
    antigen theme robbyrussell &&\
    antigen apply"

# Get Rust
RUN curl https://sh.rustup.rs -sSf | bash -s -- -y
RUN echo 'source ${HOME}/.cargo/env' >> ${HOME}/.zshrc
