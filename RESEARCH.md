# Vectorization Software: Technical Research & Architecture Guide

This document covers how raster-to-vector conversion actually works, what the best tools do under the hood, what's hard, what's unsolved, and what stack to use to build a competitive vectorizer.

---

## Table of Contents

1. [The Vectorization Pipeline](#the-vectorization-pipeline)
2. [Core Algorithms in Existing Tools](#core-algorithms-in-existing-tools)
3. [Mathematical Foundations](#mathematical-foundations)
4. [What Makes Vectorization Hard](#what-makes-vectorization-hard)
5. [ML/AI-Based Approaches](#mlai-based-approaches)
6. [State of the Art (2024-2025)](#state-of-the-art-2024-2025)
7. [Language & Stack Recommendation](#language--stack-recommendation)
8. [Architecture Blueprint](#architecture-blueprint)
9. [Sources](#sources)

---

## The Vectorization Pipeline

Every vectorizer, classical or ML-based, follows some version of this pipeline:

```
Raster Image
  │
  ├─ 1. Preprocessing (denoise, color correct)
  ├─ 2. Color Quantization (reduce to N colors) ──── or threshold to B/W
  ├─ 3. Segmentation (identify connected color regions)
  ├─ 4. Contour Tracing (extract ordered boundary pixels)
  ├─ 5. Path Fitting (fit Bezier curves to contours)
  ├─ 6. Path Simplification (reduce point count)
  ├─ 7. Optimization (merge segments, remove artifacts)
  │
  └─ SVG / Vector Output
```

### Stage 1: Preprocessing

- **Denoising**: Gaussian blur, bilateral filtering, or non-local means. Goal is to reduce noise without destroying edges.
- **Thresholding** (for B/W): Otsu's method (global) or adaptive thresholding converts grayscale to binary. Potrace accepts only binary input.

### Stage 2: Color Quantization

For multi-color vectorization, reduce millions of pixel colors to a manageable palette:

- **Median Cut** (Heckbert 1979): Recursively splits RGB color space at the median of the largest axis. Most historically popular.
- **Octree**: Builds an octree of color distribution, prunes by merging nodes until K colors remain. Used by Inkscape's multi-color trace.
- **K-Means**: Treats pixels as 3D points in RGB, iteratively assigns to nearest centroid. Celebi (2011) showed efficient k-means outperforms most other quantizers for quality.

### Stage 3: Segmentation

After quantization, identify connected regions of each color:

- **Flood-fill**: Simple connected-component labeling per color.
- **Felzenszwalb-Huttenlocher graph segmentation**: Builds pixel adjacency graph weighted by color difference, greedily merges regions.
- **SLIC superpixels**: K-means in 5D (x, y, L, a, b) space for perceptually uniform clustering.

### Stage 4: Contour Tracing

Extract ordered boundary pixel sequences from segmented regions:

- **Moore neighborhood tracing**: Follow 8-connected boundary of a region.
- **Suzuki-Abe algorithm**: Used by OpenCV's `findContours()`. Extracts topological contour hierarchies (outer boundaries + holes).
- **Potrace's directed graph model**: Vertices at pixel corners, edges oriented with black-on-left. Handles nesting via color inversion after each path extraction.

### Stage 5: Path Fitting (Bezier Curves)

Convert pixel-boundary point sequences into smooth curves. This is the most mathematically intensive step. See [Mathematical Foundations](#mathematical-foundations) for details.

### Stage 6: Path Simplification

Reduce curve segment count without visible quality loss:

- **Ramer-Douglas-Peucker (RDP)**: Recursive divide-and-conquer. Find point with max perpendicular distance from the line connecting endpoints. If > epsilon, split and recurse. O(n²) worst case. Tends to produce **spiky** results.
- **Visvalingam-Whyatt**: Iteratively removes the point forming the smallest-area triangle. O(n log n). Produces **smoother, more natural** geometry — generally preferred for organic shapes.
- **Potrace's opticurve**: Domain-specific. Merges adjacent nearly-collinear Bezier segments if direction change < 179° and error stays below `opttolerance` (default 0.2). Reduces segment count ~40%.

### Stage 7: Optimization

- Merge overlapping or adjacent paths of the same color
- Remove degenerate paths (zero-area, self-intersecting)
- Reorder layers for optimal rendering (stacking vs cutout)

---

## Core Algorithms in Existing Tools

### Potrace (Peter Selinger, 2003)

The most widely-used open-source tracing algorithm. Embedded in Inkscape. **Monochrome only** — requires binary (B/W) input.

Four stages:

**1. Path Decomposition**: Model bitmap as a directed graph where vertices are pixel corners. Follow boundary keeping black on left until returning to start. After extracting a path, **invert all pixels inside it** — this naturally handles nested holes. Odd-depth paths are outer boundaries; even-depth paths are holes. The `turnpolicy` parameter resolves ambiguities at checkerboard pixels (MINORITY, MAJORITY, BLACK, WHITE, LEFT, RIGHT). The `turdsize` parameter (default 2) discards paths whose interior area falls below threshold.

**2. Optimal Polygon Approximation**: Dynamic programming on the contour vertices. Build a directed graph connecting indices i→j where the subpath between them can be approximated as a straight line (max vertex-to-line distance ≤ 0.5px). Penalty per segment:

```
P(i,j) = |vj - vi| × sqrt( (1/(j-i+1)) × Σ dist(vk, line(vi,vj))² )
```

DP finds minimum-cost cycle: fewest segments first, then minimum penalty. Complexity O(n×m) where m is max segment length.

**3. Bezier Curve Fitting**: Each polygon vertex classified as corner or smooth using parameter `alpha = 4γ/3` where γ is the ratio of adjacent segment lengths. If alpha > `alpha_max` (default 1.0) → corner (cubic Bezier). Otherwise → smooth (quadratic Bezier). Control points placed symmetrically using alpha.

**4. Opticurve Optimization**: Merge adjacent nearly-collinear segments. Controlled by `opttolerance`.

### VTracer (Rust, visioncortex)

Handles **color images directly** — no pre-binarization needed. Notable for O(n) clustering (vs traditional O(n²)).

Pipeline:
1. **Color Clustering**: O(n) algorithm via `color_precision_loss` (1-8 bits/channel RGB quantization), `layer_difference` (gradient separation), `filter_speckle_area` (remove small patches).
2. **Two Layering Modes**: Stacked (painter's algorithm, compact SVG) or Cutout (non-overlapping shapes).
3. **Path Tracing**: Three output modes — Pixel (preserves pixelation), Polygon (straight segments), Spline (smooth Bezier).
4. **Simplification**: `corner_threshold`, `splice_threshold`, `segment_length` parameters.

### AutoTrace

GPL-licensed. Key differentiator: supports **both outline and centerline tracing**. Centerline tracing finds the medial axis/skeleton of strokes — critical for line drawings where you want single-stroke paths, not filled outlines.

### Adobe Illustrator Image Trace

Proprietary. What's known:
- Color quantization → contour tracing → Bezier fitting (likely Potrace-derived or custom)
- Separate modes for different content types (high-fidelity photo, line art, silhouettes, etc.)
- Key parameters: Threshold, Paths (smoothness vs accuracy), Corners, Noise (min region area)
- Version 29.0 (2024) "Image Trace 2.0" added significantly improved curves with fewer anchor points — possibly ML-enhanced
- Almost certainly C++ core

### Vectorizer.AI

Commercial. Claims sub-pixel precision via "Deep Vector Engine" — neural networks + classical geometry. Fits full geometric shapes (circles, ellipses, rounded rectangles, stars), not just Beziers. Detects mirror/rotational symmetries. GPU-accelerated. Currently the best automated quality available.

---

## Mathematical Foundations

### Bezier Curve Fitting

The core problem: given ordered points {P₀, P₁, ..., Pₙ} from a pixel boundary, find cubic Bezier control points (C₀, C₁, C₂, C₃) minimizing:

```
E = Σᵢ ‖B(tᵢ) - Pᵢ‖²
```

where `B(t) = (1-t)³C₀ + 3t(1-t)²C₁ + 3t²(1-t)C₂ + t³C₃`

This is **nonlinear** because the parameter values tᵢ depend on the curve itself.

#### Schneider's Algorithm (Graphics Gems, 1990)

The most widely-used general-purpose fitter:

1. **Chord-length parameterization**: Assign tᵢ proportional to cumulative arc length. Normalize to [0,1].
2. **Least-squares fit**: Build 2×2 linear system from Bernstein basis function contributions. Solve for scaling factors αₗ, αᵣ. Place interior control points at `P₁ = start + αₗ × leftTangent`, `P₂ = end + αᵣ × rightTangent`.
3. **Error evaluation**: Find max perpendicular distance from any point to fitted curve.
4. **Newton-Raphson reparameterization**: If error is moderate (within 4× tolerance), iteratively improve parameter estimates.
5. **Recursive splitting**: If error still exceeds tolerance, split at max-deviation point and recurse on each half.

#### Raph Levien's Approach (2021)

More mathematically sophisticated. Normalizes problem to span (0,0)→(1,0) with tangent angles θ₀ and θ₁. Uses Green's theorem to derive a signed-area constraint reducing 2D search to 1D. Finds x-moment zero-crossing by solving a **quartic polynomial** — giving an exact (non-iterative) solution. Guarantees that fitting a cubic Bezier to itself reproduces the identical curve.

### Corner Detection

Incorrectly smoothing a corner or cornering a smooth curve both produce visible artifacts. Approaches:

- **Potrace's alpha method**: Geometric, based on ratio of adjacent polygon segment lengths.
- **Curvature-based**: Discrete curvature at each contour point; peaks above threshold = corners.
- **Schneider's approach**: Pre-process to find tangent direction discontinuities, split curve at corners, fit each sub-curve independently.

---

## What Makes Vectorization Hard

### Anti-Aliased Edges

Aliased edges (hard pixel boundaries) are actually easy — the boundary is unambiguous. Anti-aliased edges are the real problem. Transition pixels have intermediate colors that blur the boundary between regions. A naive vectorizer either:
- Includes the AA fringe as its own thin color region (thousands of sliver shapes), or
- Assigns fringe pixels to one side, shifting the perceived boundary.

Recovering the intended **sub-pixel boundary** from anti-aliased pixels is an ill-posed inverse problem. Vectorizer.AI claims to solve this with neural networks. Academic work treats it as deconvolution.

### Gradients and Complex Fills

Continuous-tone gradients cannot be represented as flat-filled vector regions. Options and their problems:

| Approach | Problem |
|----------|---------|
| Posterize to N bands | Visible banding artifacts |
| SVG mesh gradients | Automatic fitting is unsolved in most tools |
| SVG filter effects | Fragile, not widely supported |
| Many thin shapes | Enormous file sizes |

This is why vectorizers work far better on **flat-color artwork** than photographs.

### Photographs vs Illustrations

**Logos/illustrations**: Small number of flat colors, clear edges, simple geometry. Ideal. A good vectorizer produces near-perfect reproduction with compact SVG.

**Photographs**: Continuous tones, noise, texture, millions of colors. Requires aggressive quantization (destroys detail) or enormous path counts (destroys file size). The result is always a **stylized interpretation**, never a faithful reproduction. Photo vectorization is fundamentally a lossy artistic transformation.

**Technical drawings/CAD**: Precise geometry (lines, arcs, circles) with scan noise. Benefit from specialized tools that fit geometric primitives (circles, arcs, polylines) rather than free-form Beziers.

### Performance at Scale

- Potrace DP: O(n×m) per contour — scales well but slow for images with millions of boundary pixels
- Schneider's fitting: O(n) per segment, can degrade with recursive splitting
- DiffVG optimization: minutes to hours per image
- LIVE: ~30 min/image on an RTX A5000
- VTracer's O(n) clustering: designed for scale, sub-second for typical images
- Potrace: sub-second for typical images

---

## ML/AI-Based Approaches

### Optimization-Based (Slow, High Quality)

**DiffVG** (Li et al., SIGGRAPH Asia 2020): The first differentiable vector graphics rasterizer. Not a vectorizer itself — it's infrastructure. Initialize random Bezier paths → render via DiffVG → compute pixel loss against target → backpropagate to update curve parameters → repeat. Handles the fundamental discontinuity of curve boundaries in pixel space via anti-aliasing formulations. GPU required. Slow (hundreds-thousands of iterations). Open source.

**LIVE** (Ma et al., CVPR 2022 Oral): Layer-wise Image Vectorization. Progressively adds closed Bezier paths coarse-to-fine. Uses DiffVG as renderer. Key innovations:
- UDF Loss: focuses optimization on boundary regions with highest error
- Xing Loss: penalizes self-intersecting paths
- Component-wise initialization: places new paths where error is highest

~30 min/image. Produces compact, semantically meaningful layered SVGs. Open source.

**Bezier Splatting** (NeurIPS 2025, with Adobe Research): Represents Bezier curves as 2D Gaussian splats. **30× faster forward, 150× faster backward** vs DiffVG. Includes adaptive pruning/densification (inspired by 3D Gaussian Splatting). Potential DiffVG replacement. Open source.

### Feed-Forward Neural (Fast, Limited Domain)

**Im2Vec** (CVPR 2021): VAE that outputs Bezier control points directly. Trains with only raster supervision via differentiable rendering. Fast inference (milliseconds). **Limited to simple domains**: fonts, emojis, simple icons. Output quality significantly below optimization methods.

**DeepSVG** (NeurIPS 2020, Google): Transformer-based hierarchical generative network for SVG icons. Can generate, interpolate, animate. Limited to simple icon domains. Open source.

### Text-to-SVG

**CLIPDraw**: CLIP as loss function + DiffVG as renderer. Optimizes random curves to match text prompt. Output is artistic/abstract. Non-deterministic. Not suitable for image-to-vector.

**VectorFusion** (CVPR 2023): Score Distillation Sampling from Stable Diffusion + DiffVG + LIVE. Multi-stage: generate raster → trace to SVG → fine-tune with SDS loss. Slow, oversaturated outputs (known SDS problem). Research only.

### LLM/VLM-Based (The New Direction)

This is where the field is moving. Treat SVG generation as **code generation**.

**StarVector** (CVPR 2025): Built on StarCoder (code LLM) + ViT image encoder. Outputs SVG markup directly — can use full SVG vocabulary (circles, rects, text, gradients), not just Beziers. Available as `starvector-8b-im2svg` on HuggingFace. Seconds per image. Open source.

**OmniSVG** (NeurIPS 2025): Built on Qwen-VL. First end-to-end multimodal SVG generator. Parameterizes SVG commands as discrete tokens. Scales from icons to complex anime characters. Introduces MMSVG-2M dataset (2M annotated SVGs). Open source.

**Chat2SVG** (CVPR 2025): Hybrid LLM + diffusion. LLM generates SVG skeleton → VAE path optimizer refines with diffusion guidance (SDEdit, ControlNet) → SAM identifies missed regions. Open source.

**SVGFusion** (2024): Diffusion in a **learned vector latent space** — not pixel space. Vector-Pixel Fusion VAE creates continuous latent space for SVGs. Diffusion Transformer generates in that space. Avoids slow SDS optimization entirely.

### Honest Assessment: What Actually Works in Production

| Tier | Tools | Notes |
|------|-------|-------|
| **Production today** | Vectorizer.AI, Adobe Image Trace, VTracer, Potrace | Vectorizer.AI is best quality but proprietary/paid |
| **Near-production** | StarVector, OmniSVG | Most promising direction; approaching production for icons/simple graphics |
| **Research only** | LIVE, VectorFusion, CLIPDraw, Im2Vec, Chat2SVG, NeuralSVG | Interesting but inconsistent quality or too slow |

### Performance Reality

| Approach | Speed | GPU | Real-time? |
|----------|-------|-----|------------|
| Potrace | Sub-second | No | Yes |
| VTracer | Sub-second | No | Yes |
| Bezier Splatting | Seconds-minutes | Yes | Getting there |
| StarVector/OmniSVG | Seconds | Yes | Near |
| Im2Vec inference | Milliseconds | Yes (small) | Yes |
| DiffVG optimization | Minutes | Yes | No |
| LIVE | ~30 min | Yes | No |
| VectorFusion | 10+ min | Yes (heavy) | No |

---

## State of the Art (2024-2025)

The field is in transition:

1. **Classical methods** (Potrace, VTracer) are fast and reliable for their domains but hit a quality ceiling on complex images.
2. **Optimization methods** (DiffVG/LIVE family) produce better results but are too slow for production.
3. **Bezier Splatting** (2025) may bridge this gap — 30-150× faster than DiffVG while maintaining quality.
4. **LLM-based methods** (StarVector, OmniSVG) are the most promising new direction, treating SVG as code and leveraging language model capabilities. Not yet reliable for complex images.
5. **Nobody reliably handles photographs.** This remains an artistic transformation, not a faithful conversion.

The winning approach for a new vectorizer is likely a **hybrid**: classical pipeline for the backbone (it's fast and predictable), with ML components plugged in at specific stages (edge detection, corner classification, anti-alias recovery, color segmentation).

---

## Language & Stack Recommendation

### What existing tools are built with

| Tool | Language | Why |
|------|----------|-----|
| Potrace | C | Portability, simplicity. ~5,000 lines of core logic. |
| Inkscape | C++ (wraps Potrace/AutoTrace) | Application shell. Vectorization is delegated. |
| VTracer | Rust | Memory safety, Wasm target, one codebase → CLI + lib + Python + browser |
| AutoTrace | C | Era-appropriate. Aging codebase. |
| Adobe Illustrator | C++ | Industry standard. CUDA access. CGAL-class geometry. |
| Vectorizer.AI | Unknown (GPU-heavy) | Neural + classical hybrid. |

### The honest language comparison

#### Rust — Recommended for core engine

**Real advantages:**
- Memory safety without GC — genuinely valuable for image processing where buffer overflows in pixel manipulation are a real bug class
- Fearless concurrency via Rayon — `par_iter()` makes parallelizing per-tile/per-channel work trivial
- WebAssembly is a first-class target — `wasm-pack` is mature, Photon demonstrates 4-10× over JS in browser
- One codebase → CLI + library + Python (PyO3) + Node (napi-rs) + Wasm. VTracer proves this works.
- Cargo ecosystem is a genuine productivity advantage over C++ build systems
- SIMD: stable portable SIMD as of Rust 1.80+. Auto-vectorization competitive with C++.
- GPU via wgpu: cross-platform (Vulkan/Metal/DX12/WebGPU) from one API

**Real disadvantages:**
- **Computational geometry ecosystem is immature.** No CGAL equivalent. If you need robust polygon booleans, Voronoi diagrams, or Delaunay triangulation with exact arithmetic, you write it yourself or FFI to CGAL/Clipper2. This is a real gap.
- **Image processing libraries are adequate but not deep.** `image` + `imageproc` cover basics. Nothing approaches OpenCV's breadth. You'll FFI to OpenCV for advanced operations.
- **Borrow checker friction** is real with graph-based data structures (which vectorization algorithms use heavily).
- Smaller talent pool than C++.

#### C++ — The quality ceiling option

**Real advantages:**
- **CGAL exists.** Robust exact-arithmetic computational geometry. Would take person-years to replicate.
- **OpenCV is native.** Full API, no FFI overhead, decades of optimization.
- **CUDA/TensorRT are native.** The entire NVIDIA ML inference stack assumes C++.
- Every serious graphics tool (Adobe suite, Blender, GIMP, Krita) is C++.

**Real disadvantages:**
- Memory safety is your problem. C++ vectorizers historically have CVEs.
- Build systems (CMake/Conan/vcpkg) are painful. Cross-compilation is painful.
- WebAssembly via Emscripten works but is significantly more friction than Rust.
- Concurrency is manual — no Rayon equivalent.

#### C — No. Don't.

Only relevant as algorithmic reference (Potrace). For a new project in 2026, C offers no advantages over Rust or C++ and significant disadvantages.

#### Python — Not for the core. Essential for ML.

50-100× slower than Rust/C++ for pixel-level operations. Use it for: ML model training, pipeline orchestration, prototyping algorithms, user-facing bindings. The standard pattern is Python wrapper around Rust/C++ core.

#### Go — No.

`image.At()` per-pixel overhead kills performance. GC causes latency spikes. You end up FFI-ing to C libraries anyway.

#### Zig — Not yet.

Language is well-designed for this domain but ecosystem is embryonic. No image processing, computational geometry, or ML inference libraries. Revisit in 2-3 years.

### Critical libraries by ecosystem

**Image Processing:**

| Language | Library | Assessment |
|----------|---------|------------|
| Rust | `image`, `imageproc` | Good I/O, basic filters. No advanced morphology. |
| Rust | `kornia-rs` | 3D vision, tensor system. 3-5× faster than alternatives. Newer. |
| Rust | `opencv-rust` | Full OpenCV API via FFI. Build pain. |
| C++ | OpenCV | Everything. Complex build, large dependency. |

**Computational Geometry:**

| Language | Library | Assessment |
|----------|---------|------------|
| C++ | CGAL | Gold standard. Exact arithmetic, comprehensive. |
| C++ | Clipper2 | Fast polygon booleans. Simpler than CGAL. |
| Rust | `geo` crate | Basic 2D geometry. No exact arithmetic. |
| Rust | `spade` | Delaunay triangulation. Decent. |

**GPU Compute:**

| Framework | Language | Backends |
|-----------|----------|----------|
| wgpu | Rust | Vulkan, Metal, DX12, WebGPU |
| rust-gpu | Rust | SPIR-V (write shaders in Rust) |
| CUDA | C++ | NVIDIA only |

**ML Inference:**

| Runtime | Rust Support | Notes |
|---------|-------------|-------|
| ONNX Runtime | `ort` crate | Mature. CUDA/TensorRT support. 3-5× faster than Python. |
| Tract | Native Rust | Pure Rust, CPU only. Good for Wasm builds. |
| Candle | Native Rust | HuggingFace's Rust ML framework. GPU support. |

### The actual recommendation

**Rust core, with strategic C/C++ FFI where the ecosystem demands it.**

1. Core vectorization engine in Rust (path tracing, curve fitting, color clustering)
2. `image` + `imageproc` for image I/O and basic processing
3. FFI to Clipper2 (C++) for robust polygon boolean operations
4. `ort` (ONNX Runtime) for ML model inference with GPU acceleration
5. `wgpu` for GPU-accelerated preprocessing (color quantization, edge detection)
6. `tract` as fallback ML runtime for WebAssembly builds (pure Rust, no C deps)
7. PyO3 for Python bindings
8. wasm-pack for browser deployment

This gives ~80% of a pure C++/CGAL quality ceiling at roughly half the development effort, with dramatically better cross-platform and WebAssembly support.

---

## Architecture Blueprint

### Library-first design (non-negotiable)

Build the core as a **library with a C ABI**, then layer everything on top:

```
┌─────────────────────────────────────────────┐
│                   Consumers                  │
│  CLI  │  Python (PyO3)  │  Wasm  │  Node    │
├─────────────────────────────────────────────┤
│              Public Rust API                  │
├─────────────────────────────────────────────┤
│            Core Vectorization Engine          │
│  ┌──────────┐ ┌──────────┐ ┌──────────────┐ │
│  │  Classic  │ │    ML    │ │     GPU      │ │
│  │ Pipeline  │ │ Pipeline │ │  Accelerated │ │
│  │          │ │          │ │  Preprocessor│ │
│  │ Potrace- │ │ ONNX RT  │ │              │ │
│  │ style    │ │ (ort)    │ │  wgpu        │ │
│  │ tracing  │ │ or Tract │ │  compute     │ │
│  │ + curve  │ │ for edge │ │  shaders     │ │
│  │ fitting  │ │ detect,  │ │              │ │
│  │          │ │ segment  │ │              │ │
│  └──────────┘ └──────────┘ └──────────────┘ │
├─────────────────────────────────────────────┤
│  FFI Layer: Clipper2 (polygon bools)         │
│             OpenCV (optional, advanced ops)   │
└─────────────────────────────────────────────┘
```

A monolithic application locks you out of programmatic use, which is where most professional vectorization happens (batch processing, CI pipelines, web services).

### Suggested module structure

```
vectorize/
├── crates/
│   ├── vectorize-core/       # The engine. No I/O, no filesystem, pure transforms.
│   │   ├── src/
│   │   │   ├── preprocess/   # Denoise, threshold, color quantize
│   │   │   ├── segment/      # Region detection, contour extraction
│   │   │   ├── trace/        # Boundary following, path construction
│   │   │   ├── fit/          # Bezier fitting (Schneider, Levien, Potrace-style)
│   │   │   ├── simplify/     # RDP, Visvalingam, opticurve
│   │   │   ├── optimize/     # Path merging, layer ordering
│   │   │   └── output/       # SVG serialization, path data structures
│   │   └── Cargo.toml
│   ├── vectorize-gpu/        # wgpu compute shaders for preprocessing
│   ├── vectorize-ml/         # ML model inference (ort/tract)
│   ├── vectorize-ffi/        # C ABI exports
│   └── vectorize-cli/        # CLI binary
├── bindings/
│   ├── python/               # PyO3 bindings
│   ├── wasm/                 # wasm-pack target
│   └── node/                 # napi-rs bindings
├── models/                   # Trained ML models (ONNX format)
├── ml-training/              # Python — model training scripts
└── Cargo.toml                # Workspace root
```

### What to build first (priority order)

1. **B/W tracing** — Implement Potrace-style pipeline. This is the foundation. Get contour extraction → polygon fitting → Bezier curve fitting working correctly before anything else.
2. **Multi-color via quantization** — Add color quantization + per-color tracing. This gets you to VTracer-level capability.
3. **Path simplification** — Implement both RDP and Visvalingam. Let users choose.
4. **ML edge detection** — Train or use pretrained HED/BDCN model for edge detection. Run via `ort`. This is where you start beating classical-only approaches.
5. **Anti-alias recovery** — The hard problem. Neural network trained to predict sub-pixel boundaries from anti-aliased input. This is what separates Vectorizer.AI from everything else.
6. **Geometric primitive fitting** — Detect and fit circles, ellipses, rectangles, rounded rectangles. Not just Beziers. Critical for logos and technical drawings.
7. **GPU acceleration** — Move color quantization and edge detection to wgpu compute shaders.
8. **LLM-based SVG generation** — Explore StarVector/OmniSVG integration for complex scene vectorization.

### What NOT to waste time on

- **Photo-realistic vectorization** — This is a dead end for quality. The output will always be a stylized interpretation. Focus on illustrations, logos, icons, and technical drawings first.
- **Custom ML training infrastructure** — Use PyTorch for training, export to ONNX, infer with `ort`. Don't build training in Rust.
- **GUI before the engine works** — The library is the product. CLI and programmatic access first. GUI can come later or be a separate project.
- **Gradient mesh fitting** — Unsolved in the general case. Skip until the core is solid.

---

## Sources

### Papers & Algorithms
- [Potrace: a polygon-based tracing algorithm (Selinger, 2003)](https://potrace.sourceforge.net/potrace.pdf)
- [An Algorithm for Automatically Fitting Digitized Curves (Schneider, 1990)](https://lhf.impa.br/cursos/tmg/Schneider-1990.pdf)
- [Fitting cubic Bezier curves (Raph Levien, 2021)](https://raphlinus.github.io/curves/2021/03/11/bezier-fitting.html)
- [DiffVG: Differentiable Vector Graphics Rasterization (Li et al., 2020)](https://people.csail.mit.edu/tzumao/diffvg/)
- [LIVE: Layer-wise Image Vectorization (Ma et al., CVPR 2022)](https://ma-xu.github.io/LIVE/)
- [Im2Vec (Reddy et al., CVPR 2021)](https://arxiv.org/abs/2102.02798)
- [VectorFusion (Jain et al., CVPR 2023)](https://ajayj.com/vectorfusion/)
- [Bezier Splatting (NeurIPS 2025)](https://arxiv.org/abs/2503.16424)
- [StarVector (CVPR 2025)](https://github.com/joanrod/star-vector)
- [OmniSVG (NeurIPS 2025)](https://github.com/OmniSVG/OmniSVG)
- [Chat2SVG (CVPR 2025)](https://github.com/kingnobro/Chat2SVG)
- [Image Vectorization: a Review (arXiv 2306.06441)](https://arxiv.org/abs/2306.06441)

### Open Source Implementations
- [Potrace](https://potrace.sourceforge.net/)
- [VTracer](https://github.com/visioncortex/vtracer)
- [AutoTrace](https://github.com/autotrace/autotrace)
- [DiffVG](https://github.com/BachiLi/diffvg)
- [LIVE](https://github.com/Picsart-AI-Research/LIVE-Layerwise-Image-Vectorization)
- [DeepSVG](https://github.com/alexandre01/deepsvg)
- [PyTorch-SVGRender (unified framework)](https://github.com/ximinng/PyTorch-SVGRender)
- [Schneider's FitCurves.c](https://github.com/erich666/GraphicsGems/blob/master/gems/FitCurves.c)

### Libraries Referenced
- [wgpu](https://wgpu.rs/) — Cross-platform GPU compute
- [ort (ONNX Runtime for Rust)](https://ort.pyke.io/)
- [Clipper2](https://github.com/AngusJohnson/Clipper2) — Polygon boolean operations
- [CGAL](https://www.cgal.org/) — Computational geometry
- [kornia-rs](https://github.com/kornia/kornia-rs) — Rust computer vision
