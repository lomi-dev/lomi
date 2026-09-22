ZDOTDIR=${LOMI_ZDOTDIR:-$HOME}
if [[ -f "$ZDOTDIR/.zshrc" ]]; then
  source "$ZDOTDIR/.zshrc"
fi

autoload -Uz add-zsh-hook
__lomi_preexec() { printf '\033]133;C\007'; }
__lomi_precmd() {
  local exit_code=$?
  local directory=${PWD//\%/%25}
  directory=${directory//\#/%23}
  directory=${directory// /%20}
  directory=${directory//\?/%3F}
  printf '\033]133;D;%s\007\033]7;file://localhost%s\007' "$exit_code" "$directory"
}
add-zsh-hook preexec __lomi_preexec
add-zsh-hook precmd __lomi_precmd
PROMPT=$'%{\e]133;A\a%}'"$PROMPT"$'%{\e]133;B\a%}'
