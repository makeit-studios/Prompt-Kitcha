$promptKitchaExecutable = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\src-tauri\target\release\promptkitcha.exe'))
if (-not (Test-Path -LiteralPath $promptKitchaExecutable -PathType Leaf)) {
    throw 'Build PromptKitcha first. The external-editor executable was not found.'
}
$promptKitchaEditorCommand = '"' + $promptKitchaExecutable + '" --opencode-editor'
$env:VISUAL = $promptKitchaEditorCommand
$env:EDITOR = $promptKitchaEditorCommand
Write-Output 'PromptKitcha is the external editor for OpenCode launched from this PowerShell session.'
Write-Output 'Start OpenCode, then press Ctrl+X followed by E. Enter in PromptKitcha returns the edited draft; Escape keeps the original.'
