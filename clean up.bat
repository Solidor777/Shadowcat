@echo off
rem Permanently deletes every merged, clean worktree under .worktrees.
rem Usage: "clean up.bat" [target-branch]   (default: the branch checked out here)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\clean-up-worktrees.ps1" %*
