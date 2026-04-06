// ── WASM Module Loading ──

let wasm = null;

async function loadWasm() {
  const status = document.getElementById("status");
  status.textContent = "Loading WASM engine...";
  status.className = "status";

  try {
    const mod = await import("./pkg/vectorize_wasm.js");
    await mod.default(); // init wasm
    wasm = mod;
    status.textContent = "Engine ready.";
    status.className = "status success";
    setTimeout(() => { status.textContent = ""; }, 2000);
  } catch (e) {
    status.textContent = "Failed to load WASM: " + e.message;
    status.className = "status error";
    console.error("WASM load error:", e);
  }
}

loadWasm();

// ── Mode presets (must match Rust quality.rs defaults) ──

const MODE_DEFAULTS = {
  Logo:         { color_detail: 40, path_precision: 100, curve_smoothness: 25, noise_filter: 30, gradient_layers: 15 },
  Illustration: { color_detail: 50, path_precision: 60,  curve_smoothness: 35, noise_filter: 50, gradient_layers: 30 },
  Photo:        { color_detail: 80, path_precision: 55,  curve_smoothness: 60, noise_filter: 60, gradient_layers: 60 },
  HighFidelity: { color_detail: 95, path_precision: 85,  curve_smoothness: 20, noise_filter: 20, gradient_layers: 85 },
  Sketch:       { color_detail: 25, path_precision: 70,  curve_smoothness: 15, noise_filter: 60, gradient_layers: 10 },
};

// ── State ──

let imageBytes = null;   // Uint8Array of the loaded file
let imageDataUrl = null;  // for preview
let svgString = null;     // result
let currentMode = "Logo";
let currentTab = "input";

// ── DOM refs ──

const dropzone = document.getElementById("dropzone");
const fileInput = document.getElementById("fileInput");
const btnVectorize = document.getElementById("btnVectorize");
const btnDownload = document.getElementById("btnDownload");
const statusEl = document.getElementById("status");
const previewArea = document.getElementById("previewArea");
const downloadBar = document.getElementById("downloadBar");
const svgInfo = document.getElementById("svgInfo");
const tonalToggle = document.getElementById("tonalToggle");
const tonalBody = document.getElementById("tonalBody");

// ── File loading ──

function handleFile(file) {
  if (!file || !file.type.startsWith("image/")) return;

  const reader = new FileReader();
  reader.onload = (e) => {
    imageBytes = new Uint8Array(e.target.result);

    // Also create data URL for preview
    const blob = new Blob([imageBytes], { type: file.type });
    imageDataUrl = URL.createObjectURL(blob);

    btnVectorize.disabled = false;
    svgString = null;
    downloadBar.classList.remove("visible");

    // Switch to input tab and show preview
    switchTab("input");
    showInputPreview();

    statusEl.textContent = `Loaded: ${file.name} (${formatBytes(file.size)})`;
    statusEl.className = "status";
  };
  reader.readAsArrayBuffer(file);
}

dropzone.addEventListener("click", () => fileInput.click());
fileInput.addEventListener("change", (e) => {
  if (e.target.files.length) handleFile(e.target.files[0]);
});

dropzone.addEventListener("dragover", (e) => {
  e.preventDefault();
  dropzone.classList.add("dragover");
});
dropzone.addEventListener("dragleave", () => dropzone.classList.remove("dragover"));
dropzone.addEventListener("drop", (e) => {
  e.preventDefault();
  dropzone.classList.remove("dragover");
  if (e.dataTransfer.files.length) handleFile(e.dataTransfer.files[0]);
});

// ── Mode selector ──

document.querySelectorAll(".mode-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".mode-btn").forEach((b) => b.classList.remove("active"));
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
      const valSpan = document.getElementById(`val-${key}`);
      if (valSpan) valSpan.textContent = value;
    }
  }
}

// ── Sliders ──

document.querySelectorAll("input[type='range']").forEach((slider) => {
  slider.addEventListener("input", () => {
    const valSpan = document.getElementById(`val-${slider.dataset.param}`);
    if (valSpan) valSpan.textContent = slider.value;
  });
});

// ── Tonal collapse ──

tonalToggle.addEventListener("click", () => {
  const arrow = tonalToggle.querySelector(".arrow");
  arrow.classList.toggle("open");
  tonalBody.classList.toggle("open");
});

// ── Tabs ──

document.querySelectorAll(".preview-tab").forEach((tab) => {
  tab.addEventListener("click", () => {
    switchTab(tab.dataset.tab);
  });
});

function switchTab(tab) {
  currentTab = tab;
  document.querySelectorAll(".preview-tab").forEach((t) => {
    t.classList.toggle("active", t.dataset.tab === tab);
  });

  if (tab === "input") {
    showInputPreview();
  } else {
    showOutputPreview();
  }
}

function showInputPreview() {
  if (!imageDataUrl) {
    previewArea.innerHTML = '<div class="placeholder">Load an image to get started</div>';
    return;
  }
  previewArea.innerHTML = `<img src="${imageDataUrl}" alt="Input image">`;
}

function showOutputPreview() {
  if (!svgString) {
    previewArea.innerHTML = '<div class="placeholder">Click "Vectorize" to generate SVG</div>';
    return;
  }
  previewArea.innerHTML = `<div class="svg-container">${svgString}</div>`;
}

// ── Vectorize ──

function buildConfig() {
  const quality = {};
  document.querySelectorAll("input[data-param]").forEach((slider) => {
    quality[slider.dataset.param] = parseFloat(slider.value);
  });

  return {
    mode: currentMode,
    quality,
  };
}

btnVectorize.addEventListener("click", async () => {
  if (!imageBytes || !wasm) return;

  btnVectorize.disabled = true;
  statusEl.innerHTML = '<span class="spinner"></span>Vectorizing...';
  statusEl.className = "status";
  downloadBar.classList.remove("visible");

  // Use setTimeout to let the UI update before blocking
  await new Promise((r) => setTimeout(r, 50));

  try {
    const config = buildConfig();
    const t0 = performance.now();
    svgString = wasm.vectorize(imageBytes, config);
    const elapsed = ((performance.now() - t0) / 1000).toFixed(2);

    const svgSize = new Blob([svgString]).size;
    statusEl.textContent = `Done in ${elapsed}s`;
    statusEl.className = "status success";

    svgInfo.textContent = `SVG size: ${formatBytes(svgSize)}`;
    downloadBar.classList.add("visible");

    switchTab("output");
  } catch (e) {
    statusEl.textContent = "Error: " + e.message;
    statusEl.className = "status error";
    console.error("Vectorize error:", e);
  } finally {
    btnVectorize.disabled = false;
  }
});

// ── Download ──

btnDownload.addEventListener("click", () => {
  if (!svgString) return;
  const blob = new Blob([svgString], { type: "image/svg+xml" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = "vectorized.svg";
  a.click();
  URL.revokeObjectURL(url);
});

// ── Utilities ──

function formatBytes(bytes) {
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
  return (bytes / (1024 * 1024)).toFixed(1) + " MB";
}
