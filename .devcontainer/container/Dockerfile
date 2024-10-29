FROM ubuntu:24.10

# Set locale before any packages are installed,
# since packages are influenced by the current locale on installation
#
# This is necessary for some terminal tools to work well
#  See: https://github.com/ohmyzsh/ohmyzsh/wiki/FAQ#i-see-duplicate-typed-characters-after-i-complete-a-command
RUN apt-get update && apt-get install -y language-pack-en
RUN echo LC_CTYPE=\"en_US.UTF-8\" "\n" \
    LC_ALL=\"en_US.UTF-8\" "\n" \
    LANG=\"en_US.UTF-8\" "\n" > /etc/default/locale

# Get build packages
RUN apt-get update && apt-get install -y \
    build-essential \
    curl git fish \
    vim sudo \
    gdb `#for debugging with gdb` \
    pkg-config `#for linking against e.g. libdw` \
    clang mold `#for faster compilation with mold`

# Get Node
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | sudo bash - 
RUN apt-get update && apt-get install -y nodejs

# Preload yarn with corepack
RUN corepack enable
RUN corepack prepare yarn@stable --activate


ARG USERNAME=ubuntu
ARG HOME=/home/${USERNAME}
ARG USER_UID=1000
ARG USER_GID=$USER_UID

# Create the user (disabled: user is default 'ubuntu')
# RUN groupadd --gid $USER_GID $USERNAME \
#     && useradd --uid $USER_UID --gid $USER_GID -m $USERNAME

# [Optional] Add sudo support. Omit if you don't need to install software after connecting.
RUN echo $USERNAME ALL=\(root\) NOPASSWD:ALL > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME

# Set up persistent cache directories
RUN mkdir -p \
    /cache/cargo-registry \
    /cache/cargo-toolchains \
    /cache/xdg-home
RUN chown ${USERNAME}:${USERNAME} /cache

# Start acting as the container user to avoid permissions issues
USER ${USERNAME}

# Install rust
RUN curl https://sh.rustup.rs -sSf | bash -s -- -y
COPY config.toml ${HOME}/.cargo/config.toml
RUN sudo chown ${USERNAME}:${USERNAME} ${HOME}/.cargo -R
RUN echo 'source ${HOME}/.cargo/env' >> ${HOME}/.profile

# Preinstall nightly toolchain matching the project
RUN bash -c 'source $HOME/.profile; rustup toolchain install nightly-2024-10-25-x86_64-unknown-linux-gnu\
    --profile=default\
    --component=rust-analyzer\
    --component=rustc-codegen-cranelift'

# Reconfigure xdg_home into a shared volume so fish will persist
# terminal history through rebuilds
ENV XDG_DATA_HOME="/cache/xdg-home"

# Setup fisher (a fish plugin manager)
RUN fish -c 'curl -sL https://raw.githubusercontent.com/jorgebucaran/fisher/1f0dc2b4970da160605638cb0f157079660d6e04/functions/fisher.fish | source && fisher install jorgebucaran/fisher'
# Install a theme I like with the plugin manager
RUN fish -c 'fisher install IlanCosman/tide@v5'
# Install a fenv for syncing state with other shells
RUN fish -c 'fisher install IlanCosman/tide@v5'
# Configure fish
COPY config.fish ${HOME}/fish/config.fish
# Configure the theme
RUN fish -c 'echo "121121y" | tide configure'

