$global:LomiUserPrompt = $function:prompt
function global:prompt {
  $previousSucceeded = $?
  $exitCode = if ($previousSucceeded) { 0 } else { 1 }
  $directory = ([System.Uri]$PWD.ProviderPath).AbsoluteUri
  [Console]::Write("$([char]27)]133;D;$exitCode$([char]7)$([char]27)]7;$directory$([char]7)")
  $text = & $global:LomiUserPrompt
  return "$([char]27)]133;A$([char]7)$text$([char]27)]133;B$([char]7)"
}
