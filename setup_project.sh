#!/bin/bash
set -e

# 1. Initialize Tauri App (React + TypeScript)
echo "Initializing Tauri App..."
npx create-tauri-app@latest app -- --template react-ts --manager npm -y

cd app

# 2. Install Frontend Dependencies
echo "Installing Frontend Dependencies..."
npm install lucide-react framer-motion @tanstack/react-query clsx tailwind-merge
npm install -D tailwindcss postcss autoprefixer
npx tailwindcss init -p

# 3. Add Rust Dependencies
echo "Adding Rust Dependencies..."
cd src-tauri
# Remove default to avoid conflict if needed, or just add.
# grammers-client for Telegram MTProto
# sqlx for SQLite
# tokio for async runtime
# scale-info or others if needed for grammers?
cargo add grammers-client grammers-session grammers-tl-types
cargo add tokio --features full
cargo add sqlx --features sqlite,runtime-tokio-rustls
cargo add serde --features derive
cargo add serde_json
cargo add anyhow
cargo add dirs

# 4. Install Tauri Plugins
# Note: In Tauri v2, plugins often need to be added via `tauri add` or manual cargo add + registration
echo "Adding Tauri Plugins..."
# Re-running npm install to ensure tauri CLI is available for the script if needed, 
# although npx used above should have handled it.
# We will manually add tauri-plugin-store in cargo for now as it's a common need.
cargo add tauri-plugin-store

cd ..

echo "Scaffold Complete. Run 'cd app && npm install && npm run tauri dev' to start."
