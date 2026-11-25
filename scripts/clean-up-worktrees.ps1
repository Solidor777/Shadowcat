# Sends every merged, clean worktree under .worktrees to the Recycle Bin, then prunes git's records.
#
# A worktree is removed only when all of these hold:
#   - its branch tip is an ancestor of the target branch (the first argument, or the branch checked
#     out at the project root), so no commit is lost;
#   - it is not the target branch itself;
#   - "git status --porcelain" is empty, so no uncommitted or untracked file is lost (ignored build
#     output such as target/ and node_modules/ goes to the bin with the folder).
# Anything else is skipped with the reason printed. Branches are kept.
#
# Deletion goes through the Shell's Recycle Bin verb, never a permanent delete; the bin is emptied by
# hand. The verb needs a single-threaded apartment, so this script runs under "powershell -STA".
param([string]$Target)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$wtDir = Join-Path $root ".worktrees"
if (-not (Test-Path $wtDir)) { Write-Host "No .worktrees folder at $root."; exit 0 }

if (-not $Target) { $Target = (git -C $root rev-parse --abbrev-ref HEAD).Trim() }
git -C $root rev-parse --verify --quiet "refs/heads/$Target" | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Host "Target branch '$Target' does not exist."; exit 1 }
Write-Host "Target branch: $Target"

$shell = New-Object -ComObject Shell.Application
$removed = 0; $skipped = 0
foreach ($dir in Get-ChildItem -Path $wtDir -Directory) {
  $path = $dir.FullName
  $name = $dir.Name
  if (-not (Test-Path (Join-Path $path ".git"))) { Write-Host "skip  $name  (not a worktree)"; $skipped++; continue }
  $branch = (git -C $path rev-parse --abbrev-ref HEAD).Trim()
  if ($branch -eq $Target) { Write-Host "skip  $name  (is the target branch)"; $skipped++; continue }
  $tip = (git -C $path rev-parse HEAD).Trim()
  git -C $root merge-base --is-ancestor $tip $Target
  if ($LASTEXITCODE -ne 0) { Write-Host "skip  $name  ($branch is not merged into $Target)"; $skipped++; continue }
  $dirty = git -C $path status --porcelain
  if ($dirty) { Write-Host "skip  $name  (uncommitted or untracked files)"; $skipped++; continue }

  $item = $shell.Namespace($wtDir).ParseName($name)
  if ($item) { $item.InvokeVerb("delete") }
  # The verb returns before the move finishes on a large folder; wait for it to land.
  for ($i = 0; $i -lt 600 -and (Test-Path $path); $i++) { Start-Sleep -Milliseconds 500 }
  if (Test-Path $path) { Write-Host "FAIL  $name  (still on disk after the Recycle Bin verb)"; $skipped++; continue }
  Write-Host "bin   $name  ($branch)"
  $removed++
}

git -C $root worktree prune
Write-Host "Done: $removed sent to the Recycle Bin, $skipped skipped. Empty the bin by hand."
