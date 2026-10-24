#!/usr/bin/env bash
set -euo pipefail
# Regenerate the CSS before the Rust build: static/app.css is embedded into
# the binary at compile time via include_str!, so it must exist and be current.
npm ci
npm run css:build
# No explicit `cargo build` here: the @vercel/rust builder in vercel.json
# compiles server.rs itself, and building twice only doubles deploy time.
