# Source Antigen
source ${HOME}/.antigen/antigen.zsh;
antigen use oh-my-zsh
antigen bundle git
antigen bundle heroku
antigen bundle pip
antigen bundle lein
antigen bundle command-not-found
antigen bundle zsh-users/zsh-syntax-highlighting
antigen theme awesomepanda
antigen apply

# Set up Ctrl + Backspace and Ctrl + Del so you can move around and backspace faster (try it!)
bindkey '^H' backward-kill-word
bindkey '5~' kill-word

export HISTFILE="/cache/.zsh_history"