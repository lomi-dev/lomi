if [ -f "$HOME/.bashrc" ]; then
  source "$HOME/.bashrc"
fi

__lomi_prompt() {
  local status=$?
  __lomi_ready=0
  local directory=${PWD//\%/%25}
  directory=${directory//#/%23}
  directory=${directory// /%20}
  directory=${directory//\?/%3F}
  printf '\033]133;D;%s\007\033]7;file://localhost%s\007\033]133;A\007' "$status" "$directory"
  return "$status"
}

__lomi_prompt_end() {
  local status=$?
  __lomi_ready=1
  return "$status"
}

__lomi_preexec() {
  # Bash 3.2 (shipped by macOS) has no PS0. Emit once per submitted command,
  # excluding prompt hooks, rather than once per simple command in a pipeline.
  if [[ ${__lomi_ready-0} == 1 && $1 != __lomi_prompt ]]; then
    __lomi_ready=0
    printf '\033]133;C\007'
  fi
  return 0
}

# Bash before 5.1 only executes the first PROMPT_COMMAND array member. Joining
# on newlines retains scalar hooks, including trailing comments, on every version.
__lomi_prompt_commands=(__lomi_prompt "${PROMPT_COMMAND[@]}" __lomi_prompt_end)
printf -v PROMPT_COMMAND '%s\n' "${__lomi_prompt_commands[@]}"
unset __lomi_prompt_commands

# Keep the start marker outside PS1: Readline can miscount separate invisible
# spans after a wrapped prompt is widened, moving input back into the prompt.
if (( BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4) )); then
  PS1="$PS1"'\[\e]133;B\a\]'
  PS0=$'\e]133;C\a'"${PS0-}"
elif [[ -z $(trap -p DEBUG) && $- != *T* ]] && ! shopt -q extdebug; then
  __lomi_ready=0
  trap '__lomi_preexec "$BASH_COMMAND"' DEBUG
  PS1="$PS1"'\[\e]133;B\a\]'
fi
# Preserve an existing DEBUG trap or debugger configuration on older Bash.
# Without a reliable preexec hook, omit readiness instead of authorizing MCP run.
