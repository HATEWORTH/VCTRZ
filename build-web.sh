#!/bin/bash
# Build WASM and copy to docs/ for local testing.
# Usage: ./build-web.sh
# Then serve: cd docs && python3 -m http.server 8080

set -e

echo "Building WASM..."
wasm-pack build crates/vectorize-wasm --target web --release

echo "Copying pkg to docs/..."
rm -rf docs/pkg
cp -r crates/vectorize-wasm/pkg docs/pkg

echo "Done! Serve with: cd docs && python3 -m http.server 8080"
echo "Then open http://localhost:8080"
