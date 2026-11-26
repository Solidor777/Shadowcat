@echo off
rem Permanently deletes merged, clean worktrees under .worktrees.
rem Usage: "clean up.bat" [-Target branch] [branch ...]
rem   No branch names: every merged, clean worktree. Target defaults to the branch checked out here.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\clean-up-worktrees.ps1" %*
