# VCTRZ

A high-quality raster-to-vector (image to SVG) conversion engine written in Rust. Free and open-source alternative to expensive vectorization tools and subscription services.

**Multi-target:** Desktop GUI, CLI, Rust library, Python package (PyO3), WebAssembly.

## Features

- **5 quality modes** — Logo, Illustration, Photo, High Fidelity, Sketch — each a tuned pipeline recipe
- **3 vectorization engines** — VTracer (default), Hybrid (VTracer + kurbo re-fitting + shape detection), Native (custom 7-stage pipeline)
- **8 quality axes** — Color detail, path precision, curve smoothness, noise filter, gradient layers, shadow/midtone/highlight detail (each 0-100)
- **Perceptual color science** — All color clustering in Oklab space, never raw RGB
- **Geometric shape detection** — Circles, ellipses, rectangles snapped to clean primitives
- **Line extraction** — Dual-pass vectorization separates text/outlines from color fills
- **Path merging** — Same-color regions merged with optional stroke detection
- **Parallel processing** — Rayon-based parallelism at the cluster level
- **No unsafe code**

## Quality Modes

| Mode | Best for | Key behavior |
|------|----------|-------------|
| **Logo** | Flat graphics, text, icons | Shape detection, line snapping, geometric simplification |
| **Illustration** | Cartoon, clipart | Visvalingam simplification, smooth curves, stroke detection |
| **Photo** | Photographs | Gradient preservation, tonal banding control, max color fidelity |
| **High Fidelity** | Maximum quality | Lowest tolerances, line extraction, everything enabled |
| **Sketch** | Line drawings | Line extraction, few colors, aggressive noise filter |

## Installation

### Pre-built (Desktop GUI)

Download from [Releases](https://github.com/HATEWORTH/VCTRZ/releases).

### From source

Requires [Rust 1.85+](https://rustup.rs/).

```bash
# Desktop GUI
cargo run --release -p vectorize-gui

# CLI tool
cargo install --path crates/vectorize-cli
```

### Python

```bash
pip install maturin
cd crates/vectorize-python && maturin develop --release
```

### WebAssembly

```bash
cargo install wasm-pack
wasm-pack build crates/vectorize-wasm --target web
```

## Usage

### GUI

```bash
cargo run --release -p vectorize-gui
```

Drop an image in, pick a mode, adjust sliders, export SVG.

### CLI

```bash
# Basic — uses Illustration mode by default
vectorize input.png -o output.svg

# Pick a mode
vectorize input.png -o output.svg --mode logo
vectorize input.png -o output.svg --mode photo

# Fine-tune quality axes
vectorize input.png -o output.svg --mode illustration --color-detail 80 --curve-smoothness 60

# Override engine
vectorize input.png -o output.svg --engine hybrid

# All options
vectorize input.png -o output.svg \
  --mode hifi \
  --color-detail 90 \
  --path-precision 80 \
  --noise-filter 30 \
  --gradient-layers 70 \
  --shadow-detail 50 \
  --midtone-detail 50 \
  --highlight-detail 50 \
  --colors 64 \
  --detect-shapes true \
  --extract-lines true \
  --anchor-density 50 \
  --edge-smooth 1.5

# Dump resolved config as JSON (useful for debugging)
vectorize input.png --dump-config
```

### Python

```python
import vectorize

svg = vectorize.vectorize("input.png", mode="logo")
with open("output.svg", "w") as f:
    f.write(svg)
```

### Supported Input Formats

PNG, JPEG, BMP, TIFF, WebP

## Project Structure

```
crates/
  vectorize-core/     # Pure library — all vectorization logic, no I/O
  vectorize-cli/      # CLI wrapper (clap 4)
  vectorize-gui/      # Desktop GUI (egui/eframe + resvg preview)
  vectorize-python/   # Python bindings (PyO3)
  vectorize-wasm/     # WebAssembly target (wasm-bindgen)
```

## Building

```bash
cargo build                        # Debug build (dependencies optimized)
cargo build --release              # Release build (LTO + stripped)
cargo test --workspace             # Run all tests
cargo clippy --workspace           # Lint
cargo fmt --all                    # Format
```

## How It Works

1. **Preprocess** — Denoise, threshold (Otsu), alpha handling
2. **Segment** — Color quantization in Oklab perceptual color space
3. **Trace** — Contour extraction and boundary following
4. **Fit** — Bezier curve fitting via kurbo (exact fitting, not iterative)
5. **Simplify** — Path simplification (Visvalingam-Whyatt or kurbo Bezier)
6. **Optimize** — Path merging, layer ordering (stacked vs cutout)
7. **Output** — SVG serialization with optional shape detection and line snapping

The Hybrid engine combines VTracer's mature color clustering with kurbo's curve re-fitting and geometric shape detection for the best balance of quality and editability.

## Key Dependencies

| Crate | Role |
|-------|------|
| [kurbo](https://crates.io/crates/kurbo) | Bezier curves and path operations (Raph Levien) |
| [image](https://crates.io/crates/image) | Image decoding/encoding |
| [imageproc](https://crates.io/crates/imageproc) | Edge detection, thresholding, contours |
| [palette](https://crates.io/crates/palette) | Oklab/CIELAB perceptual color science |
| [vtracer](https://crates.io/crates/vtracer) | Hierarchical color clustering engine |
| [rayon](https://crates.io/crates/rayon) | Data parallelism |
| [egui](https://crates.io/crates/egui) | Immediate-mode GUI |

## License

MIT
