@echo off
setlocal
cd /d "%~dp0"
set CLAUDE_CODE_EXPERIMENTAL_CODE_MODE=1
claude --resume
