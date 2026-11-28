@echo off
cd /d "%~dp0"
cargo build --release --target x86_64-pc-windows-msvc
pause
