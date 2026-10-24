#!/usr/bin/env bash
set -euo pipefail
npm install
# Build CSS with Tailwind before compiling the Rust binary.
# static/app.css is embedded into the binary at compile time via include_str!,
# so it must be regenerated before every Rust build.
npm run css:build
cargo build --release
