#!/bin/sh
# Build this worktree's app as a debug .app and open it through LaunchServices.
# macOS TCC reads NSMicrophoneUsageDescription and NSSpeechRecognitionUsageDescription only from
# a bundle the app launched itself, so dictation cannot be tested under `tauri dev`. Updater
# artifacts stay off: they need the release signing key.
set -eu
cd "$(dirname "$0")/.."

NAME=${PROMETEU_WORKSPACE_NAME:-}
ROOT=${PROMETEU_ROOT:-"$HOME/.prometeu-dev${NAME:+-$NAME}"}

echo "Prometeu debug bundle · raiz em $ROOT"

npm run build:app -- --debug --bundles app \
  --config src-tauri/tauri.dev.conf.json \
  --config '{"bundle":{"createUpdaterArtifacts":false}}'

# Verify the complete bundle, including Info.plist, before testing native authorization.
node scripts/terminal-state.mjs verify-bundle "src-tauri/target/debug/bundle/macos/Prometeu Dev.app"

exec open -W --env PROMETEU_ROOT="$ROOT" "src-tauri/target/debug/bundle/macos/Prometeu Dev.app"
