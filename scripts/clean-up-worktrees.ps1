# Permanently deletes every merged, clean worktree under .worktrees, then prunes git's records.
#
# The owner approves permanent deletion for this script only, and only for worktrees that pass every
# check below; it is not a general delete tool.
#
# A worktree is deleted only when all of these hold:
#   - its branch tip is an ancestor of the target branch (the first argument, or the branch checked
#     out at the project root), so no commit is lost;
#   - it is not the target branch itself;
#   - "git status --porcelain" is empty, so no uncommitted or untracked file is lost. Ignored files
#     (build output such as target/ and node_modules/) are deleted with the folder.
# Anything else is skipped with the reason printed. Branches are kept.
#
# All eligible folders go to one "rd /s /q" call. The \\?\ prefix lifts the 260-character path
# limit, which target/ and node_modules/ exceed. rd removes a junction or symbolic link inside the tree
# without following it, so pnpm's links into its shared store do not reach the store.
param([string]$Target)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$wtDir = Join-Path $root ".worktrees"
if (-not (Test-Path $wtDir)) { Write-Host "No .worktrees folder at $root."; exit 0 }

if (-not $Target) { $Target = (git -C $root rev-parse --abbrev-ref HEAD).Trim() }
git -C $root rev-parse --verify --quiet "refs/heads/$Target" | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Host "Target branch '$Target' does not exist."; exit 1 }
Write-Host "Target branch: $Target"

$eligible = @(); $skipped = 0
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
  $eligible += [pscustomobject]@{ Name = $name; Path = $path; Branch = $branch }
}

if ($eligible.Count -gt 0) {
  $args = ($eligible | ForEach-Object { '"\\?\' + $_.Path + '"' }) -join ' '
  cmd.exe /d /c "rd /s /q $args"
}

$removed = 0
foreach ($w in $eligible) {
  if (Test-Path -LiteralPath $w.Path) { Write-Host "FAIL  $($w.Name)  (still on disk)"; $skipped++ }
  else { Write-Host "gone  $($w.Name)  ($($w.Branch))"; $removed++ }
}

git -C $root worktree prune
Write-Host "Done: $removed deleted, $skipped skipped."
