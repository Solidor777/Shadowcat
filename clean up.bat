@echo off
rem Sends every merged, clean worktree under .worktrees to the Recycle Bin.
rem Usage: "clean up.bat" [target-branch]   (default: the branch checked out here)
powershell.exe -STA -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\clean-up-worktrees.ps1" %*
