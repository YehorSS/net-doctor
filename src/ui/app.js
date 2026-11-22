"use strict";

const byId = id => document.getElementById(id);
const buttons = [...document.querySelectorAll("[data-job]")];
const languages = [...document.querySelectorAll("[data-language]")];
const motion = matchMedia("(prefers-reduced-motion: reduce)");
const stageKeys = {
  improve: ["measureBefore", "adjust", "verify"],
  full: ["gateway", "internet", "download", "upload"],
  diagnose: ["gateway", "internet"],
  speed: ["idle", "download", "upload"],
};
const metrics = [
  ["gateway", "ms"], ["idlePing", "ms"], ["downPing", "ms"], ["upPing", "ms"],
  ["download", "mbps"], ["upload", "mbps"], ["loss", "percent"],
];
const results = {};
const dash = "—";
let language = localRead("language") === "en" ? "en" : "ru";
let comparison;
try {
  comparison = JSON.parse(localRead("comparison-v1"));
} catch {}
let before = comparison?.before;
let after = comparison?.after;
let revertedTrial = comparison?.revertedTrial === true;
let pending;
let latest;
let progressJob;
let progressStep = 0;
let canRestore = window.canRestore === true;
let canRestoreWifi = window.canRestoreWifi === true;
let canApply = false;

function localRead(key) {
  try { return localStorage.getItem(key); } catch { return null; }
}

function localSave(key, value) {
  try { localStorage.setItem(key, value); } catch {}
}

function status(message, state) {
  byId("status").textContent = message;
  byId("indicator").dataset.state = state;
}

for (const button of buttons) {
  button.addEventListener("click", () => {
    if (!window.ipc) {
      status(strings[language].appOnly, "error");
      return;
    }
    pending = button.dataset.job;
    progressJob = stageKeys[pending] ? pending : undefined;
    progressStep = 1;
    render();
    animateSignal();
    window.ipc.postMessage(pending);
  });
}
for (const button of languages) {
  button.addEventListener("click", () => {
    language = button.dataset.language;
    localSave("language", language);
    render();
  });
}
byId("save-baseline").addEventListener("click", () => {
  if (!after?.values.some(Number.isFinite)) return;
  before = after;
  after = undefined;
  revertedTrial = false;
  saveComparison();
  latest = { kind: "baseline" };
  render();
});

function saveComparison() {
  localSave("comparison-v1", JSON.stringify({ before, after, revertedTrial }));
}

function snapshot(run) {
  const { diagnosis, speed } = run;
  return {
    at: run.at ?? Date.now(),
    values: [
      diagnosis.local?.median, diagnosis.ping?.median,
      speed.download.Ok?.ping, speed.upload.Ok?.ping,
      speed.download.Ok?.mbps, speed.upload.Ok?.mbps, diagnosis.ping?.loss,
    ],
  };
}

window.stage = step => {
  if (!pending || !stageKeys[pending] || step <= progressStep || step > stageKeys[pending].length) return;
  progressStep = step;
  render();
  animateSignal();
};

window.report = result => {
  pending = undefined;
  latest = result;
  if (result.kind === "full" || result.kind === "improve") {
    const run = result.kind === "improve" ? (result.after ?? result.before) : result;
    results.diagnose = run.diagnosis;
    results.speed = run.speed;
    if (result.kind === "improve") {
      const original = snapshot(result.before);
      if (result.after) {
        before = original;
        after = snapshot(result.after);
        revertedTrial = result.action === "wifiReverted";
        saveComparison();
      } else if (!before && original.values.some(Number.isFinite)) {
        before = original;
        saveComparison();
      }
      canApply = false;
      delete results.optimize;
      if (result.canRestore) byId("dns-details").open = true;
    } else {
      const reading = snapshot(run);
      if (before) after = reading;
      else if (reading.values.some(Number.isFinite)) before = reading;
      revertedTrial = false;
      saveComparison();
    }
  } else {
    results[result.kind] = result;
  }
  if (["full", "improve", "diagnose"].includes(result.kind)) {
    results.idle = results.diagnose.ping;
    if (results.diagnose.https.Ok == null) byId("check-details").open = true;
  } else if (result.kind === "speed") {
    results.idle = result.idle;
  }
  if (result.canRestore != null) canRestore = result.canRestore;
  if (result.canRestoreWifi != null) canRestoreWifi = result.canRestoreWifi;
  if (result.kind === "optimize") canApply = result.message === "faster";
  if (result.kind === "apply" || result.kind === "restore") {
    canApply = false;
    delete results.optimize;
  }
  render();
};

function render() {
  const t = strings[language];
  const number = new Intl.NumberFormat(language, { maximumFractionDigits: 1 });
  const signed = new Intl.NumberFormat(language, { maximumFractionDigits: 1, signDisplay: "exceptZero" });
  const time = new Intl.DateTimeFormat(language, { dateStyle: "short", timeStyle: "short" });
  const fmt = (value, unit) => Number.isFinite(value) ? number.format(value) + (unit === "percent" ? "%" : " " + t[unit]) : dash;
  const errorText = error => {
    if (typeof error === "string") return t[error] ?? t.error;
    const [type, code] = Object.entries(error ?? {})[0] ?? [];
    return t[type]?.(code) ?? t.error;
  };
  document.documentElement.lang = language;
  for (const element of document.querySelectorAll("[data-string]")) element.textContent = t[element.dataset.string];
  for (const element of document.querySelectorAll("[data-aria]")) element.setAttribute("aria-label", t[element.dataset.aria]);
  for (const element of document.querySelectorAll("[data-title]")) element.title = t[element.dataset.title];
  for (const button of languages) button.setAttribute("aria-pressed", button.dataset.language === language);
  for (const button of buttons) {
    button.disabled = Boolean(pending);
    if (button.dataset.job === "full") button.textContent = before ? t.repeat : t.full;
    if (button.dataset.job === "apply") button.hidden = !canApply || canRestore;
    if (button.dataset.job === "restore") button.hidden = !canRestore;
    if (button.dataset.job === "restoreWifi") button.hidden = !canRestoreWifi;
  }

  const stages = byId("stages");
  stages.hidden = !progressJob;
  stages.replaceChildren(...(stageKeys[progressJob] ?? []).map((key, index) => {
    const item = document.createElement("li");
    item.textContent = t[key + "Step"];
    if (index + 1 < progressStep || (!pending && index + 1 === progressStep)) item.dataset.state = "done";
    if (pending && index + 1 === progressStep) item.setAttribute("aria-current", "step");
    return item;
  }));

  const diagnosis = results.diagnose;
  const ready = diagnosis?.https.Ok != null;
  if (diagnosis) {
    byId("gateway-address").textContent = diagnosis.gateway ?? t.noGateway;
    byId("gateway").textContent = fmt(diagnosis.local?.median, "ms");
    byId("gateway-loss").textContent = diagnosis.local ? `${t.loss} ${diagnosis.local.loss}%` : "";
    setCheck("ip", diagnosis.ip ? t.reachable : t.noReply, diagnosis.ip);
    setCheck("dns", diagnosis.dns ? t.resolved : t.error, diagnosis.dns);
    setCheck("https", ready ? fmt(diagnosis.https.Ok, "ms") : errorText(diagnosis.https.Err), ready);
  }
  const idle = results.idle;
  byId("ping").textContent = fmt(idle?.median, "ms");
  byId("loss").textContent = idle ? `${t.loss} ${idle.loss}%` : "";
  const errors = [];
  const down = results.speed?.download.Ok;
  const up = results.speed?.upload.Ok;
  for (const [direction, pingId] of [["download", "down-ping"], ["upload", "up-ping"]]) {
    const reading = results.speed?.[direction];
    byId(direction).textContent = fmt(reading?.Ok?.mbps, "mbps");
    byId(pingId).textContent = fmt(reading?.Ok?.ping, "ms");
    if (reading?.Err) errors.push(`${t[direction]} — ${errorText(reading.Err)}`);
  }
  const speedNote = byId("speed-note");
  speedNote.hidden = !errors.length && !(down?.ping === null || up?.ping === null);
  speedNote.textContent = errors.join(" ") || t.pingMissing;

  const dnsTest = results.optimize;
  const summary = dnsTest?.current != null
    ? `${t.current} ${fmt(dnsTest.current, "ms")} / Cloudflare ${fmt(dnsTest.cloudflare, "ms")}. ` : "";
  byId("benchmark").textContent = dnsTest ? summary + t[dnsTest.message] : "";

  const showComparison = Number.isFinite(before?.at) && Array.isArray(before.values);
  const hasAfter = Number.isFinite(after?.at) && Array.isArray(after.values);
  byId("comparison").hidden = !showComparison;
  byId("save-baseline").hidden = !hasAfter || !after.values.some(Number.isFinite);
  byId("save-baseline").disabled = Boolean(pending);
  if (showComparison) {
    byId("comparison-note").textContent = hasAfter
      ? t.timedRuns(time.format(before.at), time.format(after.at)) + (revertedTrial ? " " + t.trialNote : "")
      : t.baselineOnly;
    byId("comparison-rows").replaceChildren(...metrics.map(([label, unit], index) => {
      const row = document.createElement("tr");
      const title = document.createElement("th");
      title.scope = "row";
      title.textContent = t[label];
      const a = before.values[index];
      const b = hasAfter ? after.values[index] : undefined;
      const delta = Number.isFinite(a) && Number.isFinite(b) ? Math.round((b - a) * 10) / 10 : null;
      const cells = [fmt(a, unit), fmt(b, unit), delta == null ? dash : signed.format(delta) + (unit === "percent" ? " " + t.points : " " + t[unit])];
      row.append(title, ...cells.map((value, column) => {
        const cell = document.createElement("td");
        cell.textContent = value;
        if (column === 2 && delta) cell.dataset.change = (unit === "mbps" ? delta > 0 : delta < 0) ? "better" : "worse";
        return cell;
      }));
      return row;
    }));
  }

  if (pending) {
    const stage = stageKeys[pending]?.[progressStep - 1] ?? pending;
    status(t[stage + "Busy"], "busy");
  } else if (!latest) {
    status(t.idle, "idle");
  } else if (latest.kind === "full" || latest.kind === "diagnose") {
    const local = diagnosis.local;
    let tip = diagnosis.dns ? t.checkOtherSites : (diagnosis.ip ? t.tryRepair : t.checkRouter);
    if (ready) {
      if (local?.median >= 15 || (local?.median != null && local.loss && idle?.loss)) tip = t.localIssue;
      else if (latest.kind !== "full") tip = t.httpsOk;
      else if (idle?.median != null && [down?.ping, up?.ping].some(ping => ping != null && ping - idle.median >= 30)) tip = t.loadedIssue;
      else if (idle?.median >= 100 || idle?.loss >= 20) tip = t.remoteIssue;
      else if (local?.median == null || idle?.median == null || down?.ping == null || up?.ping == null) tip = t.incompletePing;
      else tip = t.healthy;
    }
    status(tip, ready ? "ok" : "error");
  } else if (latest.kind === "speed") {
    status(errors.join(" ") || t.speedDone, errors.length ? "error" : "ok");
  } else if (latest.kind === "repair") {
    status(latest.success ? t.repairDone : t.repairError, latest.success ? "ok" : "error");
  } else if (latest.kind === "optimize") {
    status(latest.current != null ? t.compareDone : t[latest.message], latest.current != null ? "ok" : "error");
  } else if (latest.kind === "baseline") {
    status(t.baselineSaved, "ok");
  } else if (latest.kind === "improve") {
    const failed = ["repairError", "wifiRestoreError", "notApplied", "dnsCheckFailed"].includes(latest.action);
    status(t[latest.action], failed ? "error" : "ok");
  } else {
    status(t[latest.message], latest.success ? "ok" : "error");
  }
}

function setCheck(id, value, ok) {
  byId(id).textContent = value;
  byId(id).dataset.state = ok ? "ok" : "error";
}

const canvas = byId("signal");
const ctx = canvas.getContext("2d");
const light = ctx.createLinearGradient(0, 0, canvas.width, 0);
light.addColorStop(0, "#79a84700");
light.addColorStop(.38, "#8abe4350");
light.addColorStop(.7, "#c4f56e");
light.addColorStop(1, "#73964930");
let signalFrame;

function drawSignal(phase) {
  const { width, height } = canvas;
  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = light;
  for (let row = 0; row < 23; row++) {
    const spread = (row - 11) / 11;
    for (let x = 0; x < width; x += 5) {
      const hill = Math.exp(-((x - 285) ** 2) / 10000);
      const ripples = Math.sin(x / 33 + spread * 3 - phase) * 22 + Math.cos(x / 62 - spread * 3 + phase / 2) * 17;
      const y = height / 2 + spread * (31 + hill * 23) + hill * ripples;
      ctx.globalAlpha = .2 + (.78 - Math.abs(spread) * .49) * ((Math.cos(x * 4 + row * 11) + 1) / 2);
      ctx.fillRect(x, Math.round(y), 2, 2);
    }
  }
}

function animateSignal() {
  cancelAnimationFrame(signalFrame);
  if (motion.matches) {
    drawSignal(0);
    return;
  }
  const start = performance.now();
  let last = -50;
  const frame = now => {
    const elapsed = now - start;
    if (elapsed - last >= 37) {
      drawSignal(1.9 * Math.sin((Math.PI * Math.min(elapsed, 3200)) / 3200));
      last = elapsed;
    }
    if (elapsed < 3200) signalFrame = requestAnimationFrame(frame);
    else drawSignal(0);
  };
  signalFrame = requestAnimationFrame(frame);
}

function boot() {
  if (motion.matches) {
    drawSignal(0);
    return;
  }
  const skull = byId("skull");
  const lines = skull.textContent.trimEnd().split("\n");
  const jaw = document.createElement("span");
  jaw.textContent = lines.slice(13).join("\n");
  skull.replaceChildren(lines.slice(0, 13).join("\n") + "\n", jaw);
  byId("boot").hidden = false;
  byId("app").inert = true;
  const finish = () => {
    byId("boot").hidden = true;
    byId("app").inert = false;
    byId("app").classList.add("arriving");
    animateSignal();
  };
  const timer = setTimeout(finish, 670);
  byId("boot").addEventListener("click", () => { clearTimeout(timer); finish(); }, { once: true });
}

function setupScrollbar() {
  const page = document.scrollingElement;
  const rail = byId("app-scrollbar");
  const thumb = byId("scroll-thumb");
  let maximum = 0;
  let travel = 0;
  let grab;

  function syncPosition() {
    const ratio = maximum ? Math.min(1, Math.max(0, page.scrollTop / maximum)) : 0;
    thumb.style.transform = `translateY(${ratio * travel}px)`;
    rail.setAttribute("aria-valuenow", Math.round(ratio * 100));
  }

  function syncSize() {
    maximum = Math.max(0, page.scrollHeight - page.clientHeight);
    rail.hidden = maximum <= 1;
    if (rail.hidden) return;
    const height = rail.clientHeight;
    const thumbHeight = Math.min(height, Math.max(32, height * page.clientHeight / page.scrollHeight));
    thumb.style.height = `${thumbHeight}px`;
    travel = height - thumbHeight;
    syncPosition();
  }

  function scrollToPointer(clientY) {
    if (travel <= 0) return;
    const railTop = rail.getBoundingClientRect().top + rail.clientTop;
    const offset = Math.max(0, Math.min(travel, clientY - railTop - grab));
    page.scrollTop = offset / travel * maximum;
  }

  rail.addEventListener("pointerdown", event => {
    if (!event.isPrimary || event.button !== 0) return;
    event.preventDefault();
    rail.focus({ preventScroll: true });
    const bounds = thumb.getBoundingClientRect();
    grab = event.target === thumb ? event.clientY - bounds.top : bounds.height / 2;
    rail.setPointerCapture(event.pointerId);
    rail.dataset.dragging = "";
    if (event.target !== thumb) scrollToPointer(event.clientY);
  });
  rail.addEventListener("pointermove", event => {
    if (grab !== undefined && rail.hasPointerCapture(event.pointerId)) scrollToPointer(event.clientY);
  });
  rail.addEventListener("lostpointercapture", () => {
    grab = undefined;
    delete rail.dataset.dragging;
  });
  rail.addEventListener("keydown", event => {
    const steps = {
      ArrowUp: -40, ArrowDown: 40,
      PageUp: -page.clientHeight, PageDown: page.clientHeight,
      Home: -maximum, End: maximum,
    };
    if (!Object.hasOwn(steps, event.key)) return;
    event.preventDefault();
    page.scrollTop += steps[event.key];
  });
  page.classList.add("app-scroll");
  window.addEventListener("scroll", syncPosition, { passive: true });
  window.addEventListener("resize", syncSize);
  new ResizeObserver(syncSize).observe(byId("app"));
  syncSize();
}

render();
setupScrollbar();
boot();
