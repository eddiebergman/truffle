#!/bin/sh
# Start from a normal Terminal window. Agents inherit this process’s OS sandbox.
cd "$(dirname "$0")" || exit 1
app="$HOME/Applications/truffle.app/Contents/MacOS/truffle-desktop"
if [ ! -x "$app" ]; then
  echo "Build/install truffle.app first. See desktop/ARCHITECTURE.md."
  exit 1
fi
PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
export PATH
exec "$app" --serve --home "$PWD"
