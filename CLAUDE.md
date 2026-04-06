# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What This Is

A high-quality raster-to-vector (image → SVG) conversion engine written in Rust. Multi-target: CLI, Rust library, Python package (PyO3), WebAssembly, desktop GUI (egui).

## Build Commands

```bash
# Build everything
cargo build

# Build and run CLI
cargo run -p vectorize-cli -- input.png -o output.svg

# Build and run desktop GUI
cargo run -p vectorize-gui

# Build only the core library
cargo build -p vectorize-core

# Run all tests
cargo test --workspace

# Run tests for a single crate
cargo test -p vectorize-core

# Run a single test by name
cargo test -p vectorize-core -- test_name

# Check without building (2-3x faster, use during dev)
cargo check --workspace

# Lint
cargo clippy --workspace

# Format
cargo fmt --all

# Build WASM (requires wasm-pack: cargo install wasm-pack)
wasm-pack build crates/vectorize-wasm --target web

# Build Python wheel (requires maturin: pip install maturin)
cd crates/vectorize-python && maturin develop
```

## Architecture

Workspace with five crates:

- **vectorize-core** — Pure library. All vectorization logic. No I/O, no filesystem in the hot path. This is the product.
- **vectorize-cli** — Thin CLI wrapper using clap 4. Handles file I/O and config.
- **vectorize-gui** — Desktop GUI using egui/eframe with resvg preview. Uses `ProgressState` for non-blocking progress reporting from the vectorizer.
- **vectorize-python** — PyO3 bindings. MUST be a separate crate (cdylib conflicts with bin targets). Always release the GIL with `py.allow_threads()` during CPU work.
- **vectorize-wasm** — wasm-bindgen. No filesystem access — all data passed as byte slices from JS. Runs single-threaded (rayon doesn't work in browser WASM without nightly + SharedArrayBuffer).

### Multi-Engine, Multi-Mode System (vectorize-core)

The core has evolved from a single pipeline into a multi-engine system controlled by quality modes.

**Three engines** (`Engine` enum):

1. **Vtracer** (default) — Wraps VTracer/visioncortex. Hierarchical color clustering, best quality for most inputs.
2. **Hybrid** — VTracer for clustering + kurbo curve re-fitting + shape detection. Best balance of quality and editability.
3. **Native** — Custom 7-stage pipeline (preprocess → segment → trace → fit → simplify → optimize → output). Used for capabilities VTracer lacks.

**Five quality modes** (`quality::Mode` enum), each a complete pipeline recipe (`ModeRecipe`):

| Mode | Use case | Engine | Key features |
|------|----------|--------|-------------|
| Logo | Flat graphics, text, icons | Hybrid | Shape detection, line snapping, geometric simplification |
| Illustration | Cartoon, clipart | Hybrid | Visvalingam simplification, smooth curves, stroke detection |
| Photo | Photographs | Vtracer | Gradient preservation, tonal banding, max color fidelity |
| HighFidelity | Maximum quality | Hybrid | Lowest tolerances, line extraction, everything enabled |
| Sketch | Line drawings | Hybrid | Line extraction, few colors, aggressive noise filter |

Logo mode has its own dedicated pipeline (`backend::logo`) that always runs regardless of engine setting.

**Eight quality axes** (`QualitySettings`, each 0-100):
`color_detail`, `path_precision`, `curve_smoothness`, `noise_filter`, `gradient_layers`, `shadow_detail`, `midtone_detail`, `highlight_detail`. These map internally to engine-specific parameters (vtracer_*, hybrid_*, native_*).

### Core Modules (vectorize-core/src/)

**Backends** (`backend/`):
- `vtracer_backend` — VTracer wrapper, maps QualitySettings to vtracer Config
- `hybrid` — VTracer + kurbo re-fitting + shape detection + selective Chaikin smoothing
- `logo` — Logo-specific pipeline with line snapping, corner sharpening, aggressive shape detection

**Pipeline stages** (used by Native engine, some shared by Hybrid):
- `preprocess` — Denoise, threshold (Otsu via imageproc), alpha handling
- `segment` — Color quantization (Oklab space), connected component labeling
- `trace` — Contour extraction, boundary following
- `fit` — Bezier curve fitting via kurbo
- `simplify` — Path simplification (Visvalingam-Whyatt or KurboBezier)
- `optimize` — Path merging, layer ordering (stacked vs cutout)
- `output` — SVG serialization from kurbo::BezPath

**Support modules**:
- `quality` — Mode system, ModeRecipe, QualitySettings with all parameter mappings
- `line_layer` — Binary line-layer extraction for text/outlines, dual-pass vectorization
- `shapes/` — Geometric primitive detection (circles, ellipses, rectangles)
- `palette` — Palette reduction (limit to N tones per hue group)
- `refine` — Adaptive curve refinement (polygon paths → smooth Beziers)
- `merge` — Same-color path merging + stroke detection

### Key Entry Points

- `vectorize(image, config)` → main entry point, dispatches to engine
- `vectorize_with_progress(image, config, state)` → GUI entry point with `ProgressState`
- `quality::build_config(mode, overrides)` → recommended way to create configs
- `post_process_svg(svg, config)` → path sanitization, snap-to-lines, fill/stroke filtering

### Key Dependencies

| Crate | Role | Why this one |
|-------|------|-------------|
| `kurbo` | Bezier curves, path ops | Raph Levien's exact fitting, not iterative approximation |
| `image` | Image I/O | Standard. Configure decoder limits for untrusted input (memory bomb prevention) |
| `imageproc` | Edge detection, contours, thresholding | Canny, Otsu, contour finding |
| `palette` | Color science | Oklab/CIELAB perceptual color distance. Critical for quality clustering |
| `rayon` | Parallelism | Each color cluster's path tracing is independent — trivially parallel |
| `geo` | 2D geometry ops | Boolean operations, containment, simplification |
| `vtracer` / `visioncortex` | Color clustering engine | Mature hierarchical clustering, default backend |
| `usvg` | SVG parsing/validation | Used by hybrid engine for re-parsing VTracer output |
| `eframe` / `egui` | GUI framework | Immediate-mode GUI for vectorize-gui |

### Key Types

- All paths use `kurbo::BezPath` — the standard Rust curve type
- All images use `image::DynamicImage` — standard ecosystem type
- All colors flow through `palette` types for perceptual correctness
- `VectorizeConfig` is the main config struct (~25 fields), best created via `quality::build_config()`

## Known Pitfalls (pre-handled in this project)

- **Debug mode is 8-10x slower** for image processing. `[profile.dev.package."*"]` is set to `opt-level = 2` so dependencies run fast even in dev.
- **Feature unification**: `resolver = "2"` is explicit in workspace Cargo.toml. Build specific packages with `-p` when testing optional features.
- **image crate memory bombs**: A 469-byte GIF can consume 7GB. Always use decoder limits for untrusted input.
- **JPEG doesn't support Rgba8**: Convert to Rgb8 before encoding JPEG output.
- **PyO3 cdylib must be separate crate**: Mixing cdylib + bin in one crate causes linker failures.
- **WASM has no filesystem**: `image::open()` won't compile for wasm32. Use `load_from_memory()`.
- **VTracer can panic**: The engine wraps VTracer calls in `catch_unwind` — always maintain this pattern.
- **Logo mode bypasses engine selection**: `vectorize_engine()` routes to `backend::logo` when mode is Logo, ignoring the engine field.
- **Windows wgpu TDR**: GPU submissions >2s get killed by Windows. Split into <100ms submissions. (Relevant when GPU features are added.)
- **Windows ort DLL conflict**: System32 ships ancient onnxruntime.dll. Use `load-dynamic` feature and `ORT_DYLIB_PATH`. (Relevant when ML features are added.)

## Design Principles

- Library-first: the core crate has no opinion about I/O, CLI, or bindings
- Standard types throughout: kurbo for curves, image for pixels, palette for color
- Perceptual color science: Oklab for all color distance/clustering, never raw RGB
- Parallel by default: rayon at the cluster-processing level
- No unsafe code (enforced by workspace lint)

## Reference Material

See `RESEARCH.md` for detailed technical documentation on:
- Vectorization algorithms (Potrace, Schneider, Levien)
- ML approaches (DiffVG, LIVE, StarVector)
- Full language/stack analysis
