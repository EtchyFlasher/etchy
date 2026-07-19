/* Etchy frontend logic.
 *
 * Runs in two modes:
 *  - TAURI: real backend via window.__TAURI__ (invoke + events)
 *  - DEMO:  plain browser — a mock backend simulates devices & flashing so
 *           the UI can be developed/showcased without hardware. */

"use strict";

// ───────────────────────── backend bridge ─────────────────────────
const IS_TAURI = !!window.__TAURI__;

const backend = IS_TAURI ? tauriBackend() : mockBackend();

function tauriBackend() {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const { open } = window.__TAURI__.dialog;
  return {
    demo: false,
    pickIso: async () => {
      const file = await open({
        multiple: false,
        filters: [{ name: "Disk images", extensions: ["iso", "img"] }],
      });
      if (!file) return null;
      return invoke("inspect_iso", { path: file });
    },
    listDevices: () => invoke("list_devices"),
    startFlash: (options) => invoke("start_flash", { options }),
    cancelFlash: () => invoke("cancel_flash"),
    onProgress: (cb) => listen("flash-progress", (e) => cb(e.payload)),
    onDone: (cb) => listen("flash-done", (e) => cb(e.payload)),
    onError: (cb) => listen("flash-error", (e) => cb(e.payload)),
    hashIso: null, // hashing happens inside the flash pipeline; also pre-shown via inspect? -> mock only
  };
}

// Mock backend: simulates everything so the GUI is fully demoable in a browser.
function mockBackend() {
  let progressCb = () => {}, doneCb = () => {}, errorCb = () => {};
  let cancelled = false;
  const devices = [
    { path: "/dev/sdb", model: "SanDisk Ultra USB 3.0", size: 30752636928, bus: "usb", removable: true, mountpoints: ["/media/user/SANDISK"] },
    { path: "/dev/sdc", model: "Kingston DataTraveler 3.0", size: 61530439680, bus: "usb", removable: true, mountpoints: [] },
  ];
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  async function runPhase(phase, total, speedBase, jitter) {
    let done = 0;
    while (done < total) {
      if (cancelled) throw new Error("cancelled");
      const speed = speedBase * (0.75 + Math.random() * jitter);
      done = Math.min(total, done + speed / 10);
      progressCb({
        phase, bytes_done: done, bytes_total: total,
        speed_bps: speed, eta_secs: Math.max(0, (total - done) / speed),
        percent: (done / total) * 100,
      });
      await sleep(100);
    }
  }

  return {
    demo: true,
    pickIso: async () => ({
      path: "/home/demo/Downloads/ubuntu-24.04.2-desktop-amd64.iso",
      name: "ubuntu-24.04.2-desktop-amd64.iso",
      size: 6114656256,
    }),
    listDevices: async () => { await sleep(300); return devices; },
    startFlash: async (options) => {
      cancelled = false;
      (async () => {
        try {
          const iso = 6114656256;
          await runPhase("hash_source", iso, 900e6, 0.4);
          if (options.badblock_check) await runPhase("bad_block_check", 2 * 1024 ** 3, 250e6, 0.5);
          await runPhase("write", iso, 42e6, 0.6);
          progressCb({ phase: "sync", bytes_done: 0, bytes_total: 1, speed_bps: 0, eta_secs: 0, percent: 0 });
          await sleep(1200);
          await runPhase("verify", iso, 95e6, 0.4);
          if (options.persistence) { progressCb({ phase: "persistence", bytes_done: 0, bytes_total: 1, speed_bps: 0, eta_secs: 0, percent: 50 }); await sleep(1500); }
          const h = "a4acfda10b18da50e2ec50ccaf860d7f20b389df8765611142305c0e911d16fd";
          doneCb({ source_sha256: h, device_sha256: h, verified: true, bytes_written: iso, elapsed_secs: 168.4, avg_write_bps: 42e6 });
        } catch {
          errorCb(t("cancelled"));
        }
      })();
    },
    cancelFlash: async () => { cancelled = true; },
    onProgress: (cb) => (progressCb = cb),
    onDone: (cb) => (doneCb = cb),
    onError: (cb) => (errorCb = cb),
  };
}

// ───────────────────────── i18n ─────────────────────────
let lang = localStorage.getItem("etchy-lang") || (navigator.language || "en").slice(0, 2);
if (!window.ETCHY_I18N[lang]) lang = "en";

function t(key) {
  return window.ETCHY_I18N[lang][key] ?? window.ETCHY_I18N.en[key] ?? key;
}
function applyI18n() {
  document.querySelectorAll("[data-i18n]").forEach((el) => (el.textContent = t(el.dataset.i18n)));
  document.querySelectorAll("[data-i18n-ph]").forEach((el) => (el.placeholder = t(el.dataset.i18nPh)));
  $("#lang-select").value = lang;
}

// ───────────────────────── state & helpers ─────────────────────────
const $ = (sel) => document.querySelector(sel);
const state = { iso: null, device: null, flashing: false, phasesSeen: [] };

const PHASES = ["hash_source", "bad_block_check", "write", "sync", "verify", "persistence", "done"];

function fmtBytes(n) {
  if (!n && n !== 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1000 && i < units.length - 1) { n /= 1000; i++; }
  return `${n.toFixed(n >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}
function fmtEta(s) {
  if (!isFinite(s) || s <= 0) return "—";
  s = Math.round(s);
  const m = Math.floor(s / 60), sec = s % 60;
  return m > 0 ? `${m}m ${String(sec).padStart(2, "0")}s` : `${sec}s`;
}

function showPanel(id) {
  ["#panel-1", "#panel-2", "#panel-3", "#panel-done"].forEach((p) => $(p).classList.add("hidden"));
  $(id).classList.remove("hidden");
  const stepNum = { "#panel-1": 1, "#panel-2": 2, "#panel-3": 3, "#panel-done": 3 }[id];
  document.querySelectorAll("#stepper .step").forEach((el) => {
    const n = +el.dataset.step;
    el.classList.toggle("active", n === stepNum);
    el.classList.toggle("done", n < stepNum);
  });
}

// ───────────────────────── step 1: ISO ─────────────────────────
async function chooseIso() {
  const info = await backend.pickIso();
  if (!info) return;
  state.iso = { ...info, sha256: null };
  $("#iso-name").textContent = info.name;
  $("#iso-size").textContent = fmtBytes(info.size);
  $("#dropzone").classList.add("hidden");
  $("#iso-card").classList.remove("hidden");
  $("#checksum-box").classList.remove("hidden");
  $("#to-step-2").disabled = false;

  // In the real app the source hash is computed during the flash pipeline
  // (phase 1) — but we show a preview hash in demo mode for the wow factor.
  if (backend.demo) {
    $("#iso-hash-status").textContent = t("hashing");
    await new Promise((r) => setTimeout(r, 1800));
    state.iso.sha256 = "a4acfda10b18da50e2ec50ccaf860d7f20b389df8765611142305c0e911d16fd";
    $("#iso-hash-status").textContent = t("hash_done");
    const el = $("#iso-hash");
    el.textContent = state.iso.sha256;
    el.classList.add("show");
    validateChecksum();
  } else {
    $("#iso-hash-status").textContent = "SHA-256 will be computed & enforced during flash";
  }
}

function clearIso() {
  state.iso = null;
  $("#dropzone").classList.remove("hidden");
  $("#iso-card").classList.add("hidden");
  $("#checksum-box").classList.add("hidden");
  $("#iso-hash").classList.remove("show");
  $("#to-step-2").disabled = true;
}

function validateChecksum() {
  const expected = $("#expected-hash").value.trim().toLowerCase();
  const verdict = $("#checksum-verdict");
  if (!expected) { verdict.textContent = ""; return; }
  if (!/^[0-9a-f]{64}$/.test(expected)) { verdict.textContent = "⚠️"; verdict.title = "not a valid SHA-256"; return; }
  if (state.iso?.sha256) {
    const ok = expected === state.iso.sha256;
    verdict.textContent = ok ? "✅" : "❌";
    verdict.title = ok ? "matches" : "DOES NOT MATCH";
  } else {
    verdict.textContent = "🔒";
    verdict.title = "will be enforced before writing";
  }
}

// ───────────────────────── step 2: drives ─────────────────────────
let refreshTimer = null;

async function refreshDrives() {
  try {
    const devices = await backend.listDevices();
    const list = $("#drive-list");
    const prevSelected = state.device?.path;
    list.innerHTML = "";
    $("#no-drives").classList.toggle("hidden", devices.length > 0);

    for (const d of devices) {
      const el = document.createElement("div");
      el.className = "drive" + (d.path === prevSelected ? " selected" : "");
      el.setAttribute("role", "option");
      el.innerHTML = `
        <span class="drive-icon">🔌</span>
        <span class="drive-body">
          <div class="drive-model"></div>
          <div class="drive-meta"></div>
        </span>
        <span class="drive-check">✓</span>`;
      el.querySelector(".drive-model").textContent = d.model || "USB Drive";
      el.querySelector(".drive-meta").textContent =
        `${d.path} · ${fmtBytes(d.size)}${d.mountpoints.length ? " · mounted" : ""}`;
      el.addEventListener("click", () => selectDrive(d, el));
      list.appendChild(el);
    }
    // If the selected drive disappeared (unplugged), deselect.
    if (prevSelected && !devices.some((d) => d.path === prevSelected)) selectDrive(null, null);
  } catch (e) {
    console.error("device enumeration failed:", e);
  }
}

function selectDrive(device, el) {
  state.device = device;
  document.querySelectorAll(".drive").forEach((d) => d.classList.remove("selected"));
  if (el) el.classList.add("selected");
  const box = $("#confirm-box");
  if (device) {
    box.classList.remove("hidden");
    $("#confirm-target").textContent = device.model;
    $("#confirm-input").value = "";
    $("#confirm-input").focus();
  } else {
    box.classList.add("hidden");
  }
  updateFlashButton();
}

function updateFlashButton() {
  const typed = $("#confirm-input").value.trim().toLowerCase();
  const target = (state.device?.model || "").trim().toLowerCase();
  $("#start-flash").disabled = !(state.iso && state.device && typed === target && target !== "");
}

// ───────────────────────── step 3: flash ─────────────────────────
const speedHistory = [];

function buildPhaseTrack(withBadblocks, withPersistence) {
  const track = $("#phase-track");
  track.innerHTML = "";
  const phases = PHASES.filter((p) =>
    (p !== "bad_block_check" || withBadblocks) &&
    (p !== "persistence" || withPersistence) &&
    p !== "done");
  for (const p of phases) {
    const chip = document.createElement("span");
    chip.className = "phase-chip";
    chip.dataset.phase = p;
    chip.textContent = t("ph_" + p);
    track.appendChild(chip);
  }
}

function setPhase(phase) {
  $("#ring-phase").textContent = t("ph_" + phase);
  document.querySelectorAll(".phase-chip").forEach((chip) => {
    const idx = PHASES.indexOf(chip.dataset.phase);
    const cur = PHASES.indexOf(phase);
    chip.classList.toggle("active", chip.dataset.phase === phase);
    chip.classList.toggle("done", idx < cur && idx !== -1);
  });
  // Ring color per phase: cyan write, violet verify, amber check.
  const colors = { write: "var(--accent)", verify: "var(--accent-2)", bad_block_check: "var(--warn)", hash_source: "var(--accent)" };
  const c = colors[phase] || "var(--accent)";
  $("#ring-fg").style.stroke = c;
  $("#ring-glow").style.stroke = c;
}

function drawSpeedGraph() {
  const canvas = $("#speed-graph");
  const ctx = canvas.getContext("2d");
  const W = canvas.width, H = canvas.height;
  ctx.clearRect(0, 0, W, H);
  if (speedHistory.length < 2) return;
  const max = Math.max(...speedHistory) * 1.15 || 1;
  const style = getComputedStyle(document.documentElement);
  const accent = style.getPropertyValue("--accent").trim();

  ctx.beginPath();
  speedHistory.forEach((v, i) => {
    const x = (i / (speedHistory.length - 1)) * W;
    const y = H - (v / max) * (H - 12) - 4;
    i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
  });
  ctx.strokeStyle = accent;
  ctx.lineWidth = 2;
  ctx.stroke();
  // glow fill
  ctx.lineTo(W, H); ctx.lineTo(0, H); ctx.closePath();
  const grad = ctx.createLinearGradient(0, 0, 0, H);
  grad.addColorStop(0, accent + "44");
  grad.addColorStop(1, accent + "00");
  ctx.fillStyle = grad;
  ctx.fill();
}

function onProgress(p) {
  const pct = p.percent ?? (p.bytes_total ? (p.bytes_done / p.bytes_total) * 100 : 0);
  const CIRC = 553; // 2πr, r=88
  $("#ring-fg").style.strokeDashoffset = CIRC - (CIRC * pct) / 100;
  $("#ring-glow").style.strokeDashoffset = CIRC - (CIRC * pct) / 100;
  $("#ring-pct").innerHTML = `${Math.floor(pct)}<span>%</span>`;
  setPhase(p.phase);
  $("#stat-speed").textContent = p.speed_bps ? fmtBytes(p.speed_bps) + "/s" : "—";
  $("#stat-eta").textContent = fmtEta(p.eta_secs);
  $("#stat-bytes").textContent = `${fmtBytes(p.bytes_done)} / ${fmtBytes(p.bytes_total)}`;
  if (p.phase === "write" || p.phase === "verify") {
    speedHistory.push(p.speed_bps || 0);
    if (speedHistory.length > 120) speedHistory.shift();
    drawSpeedGraph();
  }
}

async function startFlash() {
  const options = {
    iso_path: state.iso.path,
    device_path: state.device.path,
    expected_sha256: $("#expected-hash").value.trim() || null,
    badblock_check: $("#opt-badblocks").checked,
    persistence: $("#opt-persistence").checked
      ? { size: +$("#persist-size").value, label: $("#persist-label").value }
      : null,
  };
  state.flashing = true;
  speedHistory.length = 0;
  buildPhaseTrack(options.badblock_check, !!options.persistence);
  showPanel("#panel-3");
  clearInterval(refreshTimer);
  try {
    await backend.startFlash(options);
  } catch (e) {
    onFlashError(String(e));
  }
}

function onFlashDone(report) {
  state.flashing = false;
  showPanel("#panel-done");
  $("#result-ok").classList.remove("hidden");
  $("#result-err").classList.add("hidden");
  $("#report-details").innerHTML = `
    sha256(source) = <b>${report.source_sha256}</b><br>
    sha256(device) = <b>${report.device_sha256}</b><br>
    written: ${fmtBytes(report.bytes_written)} ·
    avg ${fmtBytes(report.avg_write_bps)}/s ·
    ${fmtEta(report.elapsed_secs)} total`;
}

function onFlashError(message) {
  state.flashing = false;
  showPanel("#panel-done");
  $("#result-ok").classList.add("hidden");
  $("#result-err").classList.remove("hidden");
  $("#error-message").textContent = message;
}

function resetToStart() {
  clearIso();
  selectDrive(null, null);
  $("#opt-badblocks").checked = false;
  $("#opt-persistence").checked = false;
  $("#persistence-opts").classList.add("hidden");
  $("#expected-hash").value = "";
  $("#checksum-verdict").textContent = "";
  showPanel("#panel-1");
}

// ───────────────────────── wiring ─────────────────────────
function init() {
  applyI18n();
  if (backend.demo) {
    $("#mode-badge").classList.remove("hidden");
    // If we're inside a Tauri window but the API is missing, something is
    // misconfigured — warn hard instead of silently pretending.
    if (navigator.userAgent.includes("Tauri") || window.__TAURI_INTERNALS__) {
      $("#mode-badge").textContent = "⚠ BACKEND UNAVAILABLE — running as demo. Reinstall or report this bug!";
    }
  }

  // theme
  const savedTheme = localStorage.getItem("etchy-theme") || "dark";
  document.documentElement.dataset.theme = savedTheme;
  $("#theme-toggle").addEventListener("click", () => {
    const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    localStorage.setItem("etchy-theme", next);
    drawSpeedGraph();
  });

  // language
  $("#lang-select").addEventListener("change", (e) => {
    lang = e.target.value;
    localStorage.setItem("etchy-lang", lang);
    applyI18n();
  });

  // step 1
  $("#dropzone").addEventListener("click", chooseIso);
  $("#dropzone").addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") chooseIso(); });
  ["dragover", "dragenter"].forEach((ev) =>
    $("#dropzone").addEventListener(ev, (e) => { e.preventDefault(); $("#dropzone").classList.add("dragover"); }));
  ["dragleave", "drop"].forEach((ev) =>
    $("#dropzone").addEventListener(ev, (e) => { e.preventDefault(); $("#dropzone").classList.remove("dragover"); }));
  $("#dropzone").addEventListener("drop", () => chooseIso()); // Tauri delivers real paths via its own drop event
  $("#iso-clear").addEventListener("click", clearIso);
  $("#expected-hash").addEventListener("input", validateChecksum);
  $("#to-step-2").addEventListener("click", () => { showPanel("#panel-2"); refreshDrives(); refreshTimer = setInterval(refreshDrives, 2500); });

  // step 2
  $("#refresh-drives").addEventListener("click", refreshDrives);
  $("#back-to-1").addEventListener("click", () => { clearInterval(refreshTimer); showPanel("#panel-1"); });
  $("#confirm-input").addEventListener("input", updateFlashButton);
  $("#opt-persistence").addEventListener("change", (e) =>
    $("#persistence-opts").classList.toggle("hidden", !e.target.checked));
  $("#start-flash").addEventListener("click", startFlash);

  // step 3
  $("#cancel-flash").addEventListener("click", () => backend.cancelFlash());
  $("#flash-another").addEventListener("click", resetToStart);
  $("#try-again").addEventListener("click", resetToStart);

  // backend events
  backend.onProgress(onProgress);
  backend.onDone(onFlashDone);
  backend.onError(onFlashError);
}

document.addEventListener("DOMContentLoaded", init);
