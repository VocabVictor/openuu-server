# Installs the repository's git hooks into .git/hooks (shared by all worktrees).
#   pwsh -File tools/git-hooks/install.ps1
$ErrorActionPreference = 'Stop'
$root = git rev-parse --path-format=absolute --git-common-dir
$hooks = Join-Path $root 'hooks'
New-Item -ItemType Directory -Force $hooks | Out-Null
Copy-Item (Join-Path $PSScriptRoot 'commit-msg') (Join-Path $hooks 'commit-msg') -Force
Write-Output ("installed commit-msg into " + $hooks)
