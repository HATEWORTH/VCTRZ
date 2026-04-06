# Vectorize — Roadmap to Industry-Leading

## Current State (March 2026)

Working prototype with:
- 5 modes (Logo, Illustration, Photo, HiFi, Sketch) as true pipeline recipes
- 3 engines (VTracer, Hybrid, Native)
- Logo-specific pipeline (B&W threshold, blur→threshold edge smoothing, VTracer spline curves, shape detection, line snapping)
- GUI with mode-aware controls (grayed out when irrelevant)
- Anchor density, edge smoothing, threshold sliders
- Snap Curves To Lines toggle, Fills/Strokes toggles
- Path/anchor/color stats in status bar
- Zoom to cursor, middle-mouse pan
- Stray line fix (force-close all subpaths + strip expansion strokes in post_process_svg)

### Known Issues
- Hybrid engine skips kurbo simplify_bezpath() due to hang bug → oversized SVGs (3-5x larger than needed)
- Non-deterministic output (k-means random init, rayon ordering)
- Contour expansion (Edge Expand slider) is dead code — strokes are added then stripped
- Native engine is not competitive with VTracer or Hybrid
- String-based SVG post-processing is fragile
- Gray/white gradient banding in Photo mode
- No live preview, no settings persistence, no batch UI

---

## Sprint 1 — Fix What's Broken (3-4 days)

### 1.1 Deterministic Output
- Seed k-means RNG with a fixed seed (or hash of image dimensions)
- Fix rayon parallel iteration ordering (use `par_iter().enumerate()` with sorted output)
- **Goal**: Same image + same settings = identical SVG every time

### 1.2 Save/Load Settings
- On app close: save all slider values, mode, engine to `~/.vectorize/settings.json`
- On app open: load and restore
- Add "Save Preset" / "Load Preset" buttons for named profiles
- Store presets in `~/.vectorize/presets/`
- **Goal**: Users never lose their workflow

### 1.3 Remove Dead Code
- Remove `contour_expansion` field from VectorizeConfig and all references
- Remove the Edge Expand slider from GUI
- Remove all stroke-adding code in hybrid.rs and output/mod.rs (it's immediately stripped anyway)
- Remove the `strip_expansion_strokes` / `nuke_expansion_strokes` post-processing (no longer needed)
- **Goal**: Cleaner codebase, fewer bugs

### 1.4 Delete Preset Enum
- Remove `quality::Preset` entirely — it's redundant with `Mode`
- Update QualitySettings to not carry a `preset` field
- Update CLI `--preset` flag to `--mode` (keep `--preset` as hidden alias)
- Update Python bindings
- **Goal**: One concept (Mode) instead of two (Mode + Preset)

### 1.5 Collapse GUI
- Add "Advanced" toggle section (collapsed by default)
- **Main controls** (always visible): Color Detail, Anchor Density, Edge Smooth, Threshold (Logo only)
- **Advanced** (collapsed): Path Precision, Curve Smooth, Noise Filter, Gradient Layers, Tonal Detail, Min Area, Tones/Hue, Simplify Method, Layer Mode, Snap To Lines, Fills/Strokes
- **Goal**: Clean UI for 90% of users, power tools for 10%

---

## Sprint 2 — Core Quality Jump (1-2 weeks)

### 2.1 Fix Kurbo Simplifier Hang
- Wrap `simplify_bezpath()` with a per-path timeout (500ms max)
- On timeout: fall back to Chaikin smoothing (current behavior)
- OR: implement Ramer-Douglas-Peucker as alternative simplifier that never hangs
- Re-enable simplification in the Hybrid engine's `refit_path()` function
- **Goal**: 2-3x smaller SVG files, production-safe Hybrid engine

### 2.2 ML Edge Detection
**This is the #1 differentiator.**

Architecture:
1. Add ONNX Runtime dependency (already mentioned in CLAUDE.md)
2. Use a pre-trained edge detection model:
   - **HED** (Holistically-Nested Edge Detection) — lightweight, fast
   - **DexiNed** — state of the art, heavier
   - **RCF** (Richer Convolutional Features) — good balance
3. Pipeline:
   ```
   Input Image
       ├── Color Layer: VTracer/Hybrid traces color regions (existing pipeline)
       └── Edge Layer: ML model → edge probability map → threshold → trace edges
   Final SVG = Color Layer + Edge Layer composited on top
   ```
4. Edge layer uses thin stroked paths (1-2px) in the detected edge color
5. User controls: Edge Strength (0-100), Edge Thickness (0.5-3px)

**Why this wins**: Adobe does this. Nobody in open-source does it well. Text stays crisp, thin lines survive, boundaries between color regions are clean instead of wobbly.

**GPU**: Run inference via ONNX Runtime with DirectML backend on Windows. Falls back to CPU if no GPU.

### 2.3 Adaptive Gradient Layers
- Replace uniform `layer_difference` with perceptual-adaptive allocation
- In achromatic (low saturation) regions: use smaller layer_difference (more layers)
- In saturated regions: use standard layer_difference (already enough layers)
- Implementation: modify the pre-quantization step in Hybrid backend to analyze image saturation map and adjust VTracer's `layer_difference` per-region
- **Goal**: Eliminate white/gray banding in Photo mode

### 2.4 Advanced Polygon Detection
- Add to shapes/mod.rs:
  - Regular polygon detection (pentagons, hexagons, octagons, stars)
  - Rotated rectangle detection (not just axis-aligned)
  - Arc/circular segment detection
- Use RANSAC-based fitting (random sample consensus)
- **Goal**: Logo mode produces clean primitives for geometric designs

---

## Sprint 3 — UX That Changes the Feel (1 week)

### 3.1 Live Low-Res Preview
- When any slider changes: immediately run vectorize at 1/4 resolution (270x270 for a 1080 image)
- Display the low-res result in the viewport
- Debounce: wait 100ms after last slider change before rendering
- On mouse release (stop dragging): run full-res vectorize
- No GPU needed — 1/4 resolution runs in <200ms on CPU
- **Goal**: Users see what they're getting in real-time

### 3.2 Side-by-Side Comparison
- Add a "Compare" toggle button next to INPUT/OUTPUT tabs
- When active: split viewport vertically, left=input, right=output
- Sync zoom and pan between both views
- Optional: draggable split bar
- **Goal**: Users can evaluate quality at any zoom level

### 3.3 PDF Export
- Add `resvg` + `pdf-writer` dependencies
- EXPORT button gets a dropdown: SVG, PDF
- PDF embeds the vector paths directly (not a rasterized SVG)
- **Goal**: Professional workflow compatibility

### 3.4 Batch Processing
- Add BATCH tab to GUI
- Drag-and-drop multiple files or select folder
- Apply current settings to all files
- Show per-file progress bar
- Output to user-selected folder with `{filename}_vectorized.svg` pattern
- **Goal**: Production users can process hundreds of files

---

## Sprint 4 — Performance (2 weeks)

### 4.1 GPU K-Means
- Implement k-means clustering as wgpu compute shader
- Each pixel independently computes distance to all centers → trivially parallel
- Center update step: parallel reduction
- **Goal**: 10x faster color quantization on large images
- **Note**: Windows TDR — keep each dispatch <100ms

### 4.2 Parallel Contour Tracing
- Native pipeline: trace contours per-label in parallel (rayon)
- Currently single-threaded bottleneck
- **Goal**: 2-4x faster native pipeline

### 4.3 SVG Output Optimization
- Replace string-based post-processing with roxmltree XML parser
- Single-pass SVG generation instead of generate → parse → modify → re-serialize
- Pre-allocate output buffer based on path count estimate
- **Goal**: Correct SVG handling, 2x faster output generation

---

## What Each Sprint Delivers

| Sprint | Time | Quality Impact | UX Impact | Performance Impact |
|--------|------|---------------|-----------|-------------------|
| 1 | 3-4 days | Minor (cleanup) | Major (settings persist, cleaner GUI) | None |
| 2 | 1-2 weeks | **Massive** (ML edges, smaller files, no banding) | Minor | Moderate (smaller SVGs) |
| 3 | 1 week | None | **Massive** (live preview, comparison, batch) | None |
| 4 | 2 weeks | None | Minor | **Massive** (GPU accel, parallel) |

---

## Competitive Position After All Sprints

| Feature | Adobe Illustrator | Vectorize (After) |
|---------|------------------|-------------------|
| Edge quality | ML-based | ML-based (same level) |
| Color science | Proprietary | Oklab (better) |
| Simplification | RDP + bezier | RDP + kurbo (comparable) |
| Shape detection | ML | RANSAC + heuristic (comparable) |
| Live preview | GPU | CPU low-res (acceptable) |
| Batch processing | Yes | Yes |
| Price | $23/month | Free / open source |
| Modes/presets | 50+ presets | 5 modes + user presets |
| Cross-platform | Mac/Win | Mac/Win/Linux/WASM |

---

## Files to Reference

| File | Purpose |
|------|---------|
| `crates/vectorize-core/src/lib.rs` | Main pipeline, VectorizeConfig, post_process_svg |
| `crates/vectorize-core/src/backend/hybrid.rs` | Hybrid engine (primary engine) |
| `crates/vectorize-core/src/backend/logo.rs` | Logo-specific pipeline |
| `crates/vectorize-core/src/backend/vtracer_backend.rs` | VTracer wrapper |
| `crates/vectorize-core/src/quality.rs` | Mode enum, ModeRecipe, QualitySettings |
| `crates/vectorize-core/src/preprocess/mod.rs` | Image preprocessing (threshold, blur, achromatic boost) |
| `crates/vectorize-core/src/segment/mod.rs` | Color quantization (Oklab k-means) |
| `crates/vectorize-core/src/fit/mod.rs` | Bezier curve fitting |
| `crates/vectorize-core/src/simplify/mod.rs` | Path simplification (KurboBezier, VW) |
| `crates/vectorize-core/src/shapes/mod.rs` | Geometric primitive detection |
| `crates/vectorize-core/src/merge.rs` | Path merging + stroke detection |
| `crates/vectorize-core/src/output/mod.rs` | SVG serialization |
| `crates/vectorize-core/src/line_layer.rs` | Line extraction (dual-pass) |
| `crates/vectorize-gui/src/main.rs` | GUI application |
| `crates/vectorize-gui/src/theme.rs` | GUI theme (Trilithium) |
| `CLAUDE.md` | Build commands, architecture, pitfalls |
| `RESEARCH.md` | Algorithm research and references |
