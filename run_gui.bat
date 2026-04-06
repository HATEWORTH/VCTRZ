@echo off
cd /d "%~dp0"
cargo run -p vectorize-gui --release
