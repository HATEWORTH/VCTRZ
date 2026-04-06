// ── WASM Module Loading ──

let wasm = null;

async function loadWasm() {
  logMsg("Loading WASM engine...");
  try {
    const mod = await import("./pkg/vectorize_wasm.js");
    await mod.default();
    wasm = mod;
    logMsg("Engine ready.");
    setStatus("Engine ready.");
  } catch (e) {
    logMsg("Failed to load WASM: " + e.message);
    setStatus("Failed to load WASM: " + e.message, true);
    console.error("WASM load error:", e);
  }
}

loadWasm();

// ── Mode presets (must match Rust quality.rs) ──

const MODE_DEFAULTS = {
  Logo:         { color_detail: 40, path_precision: 100, curve_smoothness: 25, noise_filter: 30, gradient_layers: 15, anchor_density: 50, edge_smoothing: 15, color_threshold: 20 },
  Illustration: { color_detail: 50, path_precision: 60,  curve_smoothness: 35, noise_filter: 50, gradient_layers: 30, anchor_density: 50, edge_smoothing: 15, color_threshold: 20 },
  Photo:        { color_detail: 80, path_precision: 55,  curve_smoothness: 60, noise_filter: 60, gradient_layers: 60, anchor_density: 50, edge_smoothing: 15, color_threshold: 20 },
  HighFidelity: { color_detail: 95, path_precision: 85,  curve_smoothness: 20, noise_filter: 20, gradient_layers: 85, anchor_density: 50, edge_smoothing: 15, color_threshold: 20 },
  Sketch:       { color_detail: 25, path_precision: 70,  curve_smoothness: 15, noise_filter: 60, gradient_layers: 10, anchor_density: 50, edge_smoothing: 15, color_threshold: 20 },
};

// ── State ──

let imageBytes = null;
let imageDataUrl = null;
let svgString = null;
let currentMode = "Logo";
let currentEngine = "Vtracer";
let currentTab = "input";
let busy = false;

// ── DOM refs ──

const dropzone = document.getElementById("dropzone");
const fileInput = document.getElementById("fileInput");
const btnOpen = document.getElementById("btnOpen");
const btnExport = document.getElementById("btnExport");
const btnVectorize = document.getElementById("btnVectorize");
const btnDownload = document.getElementById("btnDownload");
const btnCancel = document.getElementById("btnCancel");
const btnClear = document.getElementById("btnClear");
const viewport = document.getElementById("viewport");
const downloadBar = document.getElementById("downloadBar");
const svgInfo = document.getElementById("svgInfo");
const logBox = document.getElementById("logBox");
const statusText = document.getElementById("statusText");
const progressFill = document.getElementById("progressFill");
const advancedToggle = document.getElementById("advancedToggle");
const advancedBody = document.getElementById("advancedBody");

// ── Logging ──

function logMsg(msg) {
  const p = document.createElement("p");
  p.textContent = msg;
  logBox.appendChild(p);
  logBox.scrollTop = logBox.scrollHeight;
}

function setStatus(msg, isError) {
  statusText.textContent = msg;
  statusText.className = "status-text" + (isError ? " error" : "");
}

function setProgress(pct, isBusy) {
  progressFill.style.width = pct + "%";
  progressFill.className = "progress-fill" + (isBusy ? " busy" : "");
}

// ── File loading ──

function handleFile(file) {
  if (!file || !file.type.startsWith("image/")) return;

  const reader = new FileReader();
  reader.onload = (e) => {
    imageBytes = new Uint8Array(e.target.result);
    const blob = new Blob([imageBytes], { type: file.type });
    imageDataUrl = URL.createObjectURL(blob);

    btnVectorize.disabled = false;
    svgString = null;
    downloadBar.classList.remove("visible");

    switchTab("input");
    showInputPreview();

    logMsg(`Loaded: ${file.name} (${formatBytes(file.size)})`);
    setStatus(`${file.name} | Ready`);
  };
  reader.readAsArrayBuffer(file);
}

btnOpen.addEventListener("click", () => fileInput.click());
dropzone.addEventListener("click", () => fileInput.click());
fileInput.addEventListener("change", (e) => {
  if (e.target.files.length) handleFile(e.target.files[0]);
});

dropzone.addEventListener("dragover", (e) => { e.preventDefault(); dropzone.classList.add("dragover"); });
dropzone.addEventListener("dragleave", () => dropzone.classList.remove("dragover"));
dropzone.addEventListener("drop", (e) => {
  e.preventDefault();
  dropzone.classList.remove("dragover");
  if (e.dataTransfer.files.length) handleFile(e.dataTransfer.files[0]);
});

// ── Mode selector ──

document.querySelectorAll("[data-mode]").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll("[data-mode]").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    currentMode = btn.dataset.mode;
    applyModeDefaults(currentMode);
  });
});

function applyModeDefaults(mode) {
  const defaults = MODE_DEFAULTS[mode];
  if (!defaults) return;

  for (const [key, value] of Object.entries(defaults)) {
    const slider = document.querySelector(`input[data-param="${key}"]`);
    if (slider) {
      slider.value = value;
      updateSliderDisplay(slider);
    }
  }

  // Logo mode: enable threshold, disable color detail
  const isLogo = mode === "Logo";
  const thresholdRow = document.getElementById("thresholdRow");
  if (thresholdRow) {
    thresholdRow.classList.toggle("disabled", !isLogo);
  }
}

// ── Engine selector ──

document.querySelectorAll("[data-engine]").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll("[data-engine]").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
    currentEngine = btn.dataset.engine;
  });
});

// ── Layer mode ──

document.querySelectorAll("[data-layer]").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll("[data-layer]").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
  });
});

// ── Simplify method ──

document.querySelectorAll("[data-simplify]").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll("[data-simplify]").forEach((b) => b.classList.remove("active"));
    btn.classList.add("active");
  });
});

// ── Sliders ──

function updateSliderDisplay(slider) {
  const param = slider.dataset.param;
  const valSpan = document.getElementById(`val-${param}`);
  if (!valSpan) return;

  if (param === "edge_smoothing") {
    valSpan.textContent = (parseFloat(slider.value) / 10).toFixed(1);
  } else if (param === "tones_per_hue") {
    valSpan.textContent = slider.value === "0" ? "Off" : slider.value;
  } else {
    valSpan.textContent = slider.value;
  }
}

document.querySelectorAll("input[type='range']").forEach((slider) => {
  slider.addEventListener("input", () => updateSliderDisplay(slider));
});

// ── Checkboxes ──

document.querySelectorAll(".checkbox-row").forEach((row) => {
  row.addEventListener("click", () => {
    row.classList.toggle("checked");
    const box = row.querySelector(".checkbox-box");
    box.textContent = row.classList.contains("checked") ? "\u2713" : "";
  });
});

// ── Advanced collapse ──

advancedToggle.addEventListener("click", () => {
  const isOpen = advancedBody.classList.toggle("open");
  advancedToggle.textContent = (isOpen ? "\u25BC" : "\u25B6") + " ADVANCED";
});

// ── Tabs ──

document.getElementById("tabInput").addEventListener("click", () => switchTab("input"));
document.getElementById("tabOutput").addEventListener("click", () => switchTab("output"));

function switchTab(tab) {
  currentTab = tab;
  document.querySelectorAll("[data-tab]").forEach((t) => {
    t.classList.toggle("active", t.dataset.tab === tab);
  });
  if (tab === "input") showInputPreview();
  else showOutputPreview();
}

function showInputPreview() {
  if (!imageDataUrl) {
    viewport.innerHTML = '<div class="placeholder">Drop or open an image</div>';
    return;
  }
  viewport.innerHTML = `<img src="${imageDataUrl}" alt="Input image">`;
}

function showOutputPreview() {
  if (!svgString) {
    viewport.innerHTML = '<div class="placeholder">Click VECTORIZE to generate SVG</div>';
    return;
  }
  viewport.innerHTML = `<div class="svg-container">${svgString}</div>`;
}

// ── Build config ──

function buildConfig() {
  const quality = {};
  document.querySelectorAll("input[data-param]").forEach((slider) => {
    const param = slider.dataset.param;
    let val = parseFloat(slider.value);
    // edge_smoothing is stored as x10 int in the slider
    if (param === "edge_smoothing") val = val / 10;
    quality[param] = val;
  });

  return { mode: currentMode, quality };
}

// ── Vectorize ──

btnVectorize.addEventListener("click", async () => {
  if (!imageBytes || !wasm || busy) return;

  busy = true;
  btnVectorize.disabled = true;
  btnCancel.disabled = false;
  downloadBar.classList.remove("visible");
  setProgress(50, true);
  logMsg("Starting vectorization...");
  logMsg(`  Engine: ${currentEngine}  Mode: ${currentMode}`);
  setStatus(`Vectorizing... | ${currentMode}`);

  await new Promise((r) => setTimeout(r, 50));

  try {
    const config = buildConfig();
    const t0 = performance.now();
    svgString = wasm.vectorize(imageBytes, config);
    const elapsed = ((performance.now() - t0) / 1000).toFixed(2);

    const svgSize = new Blob([svgString]).size;
    const paths = (svgString.match(/<path/g) || []).length;

    logMsg(`  SVG generated: ${formatBytes(svgSize)} | ${paths} paths`);
    logMsg(`  Done in ${elapsed}s`);
    setStatus(`Done in ${elapsed}s | P:${paths} | ${formatBytes(svgSize)}`);
    setProgress(100, false);

    svgInfo.textContent = `${formatBytes(svgSize)} | ${paths} paths`;
    downloadBar.classList.add("visible");
    btnExport.disabled = false;

    switchTab("output");
  } catch (e) {
    logMsg("ERROR: " + e.message);
    setStatus("Error: " + e.message, true);
    setProgress(0, false);
    console.error("Vectorize error:", e);
  } finally {
    busy = false;
    btnVectorize.disabled = !imageBytes;
    btnCancel.disabled = true;
  }
});

// ── Clear ──

btnClear.addEventListener("click", () => {
  imageBytes = null;
  imageDataUrl = null;
  svgString = null;
  btnVectorize.disabled = true;
  btnExport.disabled = true;
  downloadBar.classList.remove("visible");
  viewport.innerHTML = '<div class="placeholder">Drop or open an image</div>';
  logBox.innerHTML = "";
  setStatus("Ready");
  setProgress(0, false);

  // Reset mode to Logo
  document.querySelectorAll("[data-mode]").forEach((b) => b.classList.remove("active"));
  document.querySelector('[data-mode="Logo"]').classList.add("active");
  currentMode = "Logo";
  applyModeDefaults("Logo");
});

// ── Download / Export ──

function downloadSvg() {
  if (!svgString) return;
  const blob = new Blob([svgString], { type: "image/svg+xml" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = "vectorized.svg";
  a.click();
  URL.revokeObjectURL(url);
}

btnDownload.addEventListener("click", downloadSvg);
btnExport.addEventListener("click", downloadSvg);

// ── Utilities ──

function formatBytes(bytes) {
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
  return (bytes / (1024 * 1024)).toFixed(1) + " MB";
}
