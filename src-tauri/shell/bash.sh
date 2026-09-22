if [ -f "$HOME/.bashrc" ]; then
  source "$HOME/.bashrc"
fi

__lomi_prompt() {
  local status=$?
  local directory=${PWD//\%/%25}
  directory=${directory//#/%23}
  directory=${directory// /%20}
  directory=${directory//\?/%3F}
  printf '\033]133;D;%s\007\033]7;file://localhost%s\007\033]133;A\007' "$status" "$directory"
  return "$status"
}

# Preserve existing prompt hooks and readline's non-printing character accounting.
PROMPT_COMMAND=(__lomi_prompt "${PROMPT_COMMAND[@]}")
# Keep the start marker outside PS1: Readline can miscount separate invisible
# spans after a wrapped prompt is widened, moving input back into the prompt.
PS1="$PS1"'\[\e]133;B\a\]'
PS0=$'\e]133;C\a'"${PS0-}"
