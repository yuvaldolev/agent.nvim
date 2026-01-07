#!/bin/bash
# Run all Lua unit tests for the Neovim plugin

set -e

cd "$(dirname "$0")/../.."

echo "=== Running Spinner Tests ==="
lua nvim-plugin/tests/spinner_spec.lua

echo ""
echo "=== Running Init Tests ==="
lua nvim-plugin/tests/init_spec.lua

echo ""
echo "=== All Tests Passed ==="
