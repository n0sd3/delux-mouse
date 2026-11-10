// Delux Mouse — settings window. Talks to the Rust side through Tauri commands.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (sel) => document.querySelector(sel);
const el = (tag, props = {}, ...kids) => {
  const e = Object.assign(document.createElement(tag), props);
  for (const k of kids) e.append(k);
  return e;
};

let state = null;      // last UiState from Rust
let draft = null;      // edited copy of the active profile
let tab = "sensor";
let note = null;       // { text, err } last outcome, shown in the header line
let pushTimer = null;  // debounce for sending edits
let pushing = false;   // an edit is on its way to the mouse

const POLLING = [["Hz125", "125 Hz"], ["Hz250", "250 Hz"], ["Hz500", "500 Hz"], ["Hz1000", "1000 Hz"],
  ["Hz2000", "2000 Hz"], ["Hz4000", "4000 Hz"], ["Hz8000", "8000 Hz"]];
// Names from the vendor driver's language file (ms_lightmode1..12).
const LIGHT_MODES = ["Desligada", "Estática", "Respiração", "Neon", "Respiração colorida", "Estática colorida",
  "Respiração mista", "Onda arco-íris", "Relâmpago", "Estática mista", "Letreiro", "Letreiro 2"];

// Firmware functions offered for a button, grouped as in the vendor driver.
// Keyboard shortcuts are the Mac ones (Cmd = HID modifier 0x08, Ctrl 0x01, Shift 0x02, Alt 0x04).
const key = (modifiers, usage) => ({ Key: { modifiers, usage } });
const ACTION_GROUPS = [
  ["Mouse", [
    ["Clique esquerdo", "Left"], ["Clique direito", "Right"], ["Clique do meio", "Middle"],
    ["Voltar", "Back"], ["Avançar", "Forward"], ["Duplo clique", "DoubleClick"],
    ["Fogo (clique repetido)", "Fire"], ["Mira (DPI baixo enquanto segura)", { Raw: [0x10, 0, 3] }],
    ["Scroll para cima", "ScrollUp"], ["Scroll para baixo", "ScrollDown"],
    ["Scroll para a esquerda", { Raw: [0x0b, 0, 0] }], ["Scroll para a direita", { Raw: [0x0c, 0, 0] }],
    ["Desativado", "Disabled"],
  ]],
  ["DPI e luz", [
    ["Trocar DPI", "DpiCycle"], ["DPI +", "DpiUp"], ["DPI −", "DpiDown"], ["Trocar modo de luz", { Raw: [0x29, 0, 3] }],
  ]],
  ["Mídia", [
    ["Tocar/Pausar", { Media: 0x18 }], ["Parar", { Media: 0x19 }], ["Próxima faixa", { Media: 0x17 }],
    ["Faixa anterior", { Media: 0x16 }], ["Volume +", { Media: 0x1b }], ["Volume −", { Media: 0x1c }], ["Mudo", { Media: 0x1a }],
  ]],
  ["Navegador", [
    ["Página inicial", { Media: 0x25 }], ["Avançar página", { Media: 0x20 }], ["Voltar página", { Media: 0x21 }],
    ["Parar carregamento", { Media: 0x22 }], ["Recarregar", { Media: 0x24 }], ["Buscar", { Media: 0x26 }],
  ]],
  ["Atalhos do Mac", [
    ["Copiar (⌘C)", key(8, 0x06)], ["Colar (⌘V)", key(8, 0x19)], ["Recortar (⌘X)", key(8, 0x1b)],
    ["Desfazer (⌘Z)", key(8, 0x1d)], ["Refazer (⇧⌘Z)", key(0x0a, 0x1d)], ["Selecionar tudo (⌘A)", key(8, 0x04)],
    ["Salvar (⌘S)", key(8, 0x16)], ["Buscar (⌘F)", key(8, 0x09)], ["Fechar janela (⌘W)", key(8, 0x1a)],
    ["Trocar de app (⌘Tab)", key(8, 0x2b)], ["Spotlight (⌘Espaço)", key(8, 0x2c)],
    ["Mission Control (⌃↑)", key(1, 0x52)], ["Captura de tela (⇧⌘4)", key(0x0a, 0x21)], ["Bloquear tela (⌃⌘Q)", key(9, 0x14)],
  ]],
];

// KeyboardEvent.code → HID keyboard usage, for the "record a shortcut" button.
const HID_USAGE = (() => {
  const m = {};
  "ABCDEFGHIJKLMNOPQRSTUVWXYZ".split("").forEach((c, i) => (m["Key" + c] = 0x04 + i));
  "1234567890".split("").forEach((c, i) => (m["Digit" + c] = 0x1e + i));
  for (let i = 1; i <= 12; i++) m["F" + i] = 0x39 + i;
  Object.assign(m, { Enter: 0x28, Escape: 0x29, Backspace: 0x2a, Tab: 0x2b, Space: 0x2c, Minus: 0x2d, Equal: 0x2e,
    BracketLeft: 0x2f, BracketRight: 0x30, Backslash: 0x31, Semicolon: 0x33, Quote: 0x34, Backquote: 0x35, Comma: 0x36,
    Period: 0x37, Slash: 0x38, Home: 0x4a, PageUp: 0x4b, Delete: 0x4c, End: 0x4d, PageDown: 0x4e,
    ArrowRight: 0x4f, ArrowLeft: 0x50, ArrowDown: 0x51, ArrowUp: 0x52 });
  return m;
})();
const USAGE_NAME = Object.fromEntries(Object.entries(HID_USAGE).map(([code, u]) => [u, code.replace(/^Key|^Digit/, "").replace("Arrow", "Seta ")]));
function describeKey({ modifiers, usage }) {
  const mods = [[1, "⌃"], [4, "⌥"], [2, "⇧"], [8, "⌘"], [0x10, "⌃"], [0x40, "⌥"], [0x20, "⇧"], [0x80, "⌘"]]
    .filter(([bit]) => modifiers & bit).map(([, sym]) => sym);
  return [...new Set(mods)].join("") + (USAGE_NAME[usage] ?? `tecla 0x${usage.toString(16)}`);
}

const SCRIPT_SLOTS = [3, 6, 7]; // physical buttons a script may take on this mouse
let scriptDraft = {};   // slot -> { label, command } typed but not yet saved

const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const active = () => state.store.profiles.find((p) => p.name === state.store.active);
const unverified = (key) => state.unverified.includes(key)
  ? el("span", { className: "badge", textContent: "não verificado", title: "Enviado ao mouse sem erro, mas o efeito não foi confirmado nos testes." })
  : "";
const hex = ([r, g, b]) => "#" + [r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("");
const rgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));

async function refresh() {
  state = await invoke("get_state");
  // The working copy follows the store, except while an edit is being typed or sent.
  if (!draft || draft.name !== state.store.active || (!pushTimer && !pushing)) draft = structuredClone(active());
  else draft.scripts = structuredClone(active().scripts), draft.buttons = mergeScriptButtons(draft.buttons);
  render();
}

// Script bindings change buttons on the Rust side; keep those slots in step in the draft.
function mergeScriptButtons(buttons) {
  const stored = active();
  const out = structuredClone(buttons);
  for (let i = 0; i < out.length; i++) {
    const wasScript = out[i]?.Key && out[i].Key.modifiers === 0 && out[i].Key.usage >= 0x68;
    const isScript = stored.scripts.some((s) => s.slot === i);
    if (isScript || wasScript) out[i] = stored.buttons[i];
  }
  return out;
}

const sectionNames = { Sensor: "sensor", Polling: "polling", PowerLight: "luz/energia", Buttons: "botões", Macros: "macros" };

function statusText(s) {
  const b = s.battery ? `${s.battery[1]}%` : "";
  if (s.connection === "NoReceiver") return "Receptor não conectado";
  if (s.connection === "Asleep") return b ? `Mouse dormindo — última leitura de bateria: ${b}` : "Mouse dormindo ou desligado";
  if (!s.battery) return "Conectado";
  const st = s.battery[0];
  const extra = st === "Charging" ? " (carregando)" : st === "Full" ? " (carga completa)" : "";
  return `Conectado — bateria ${b}${extra}`;
}

function render() {
  renderChrome();
  renderTab();
}

// Header only. Safe to call on every status event: it never touches the tab,
// so nothing the user is typing is lost.
function renderChrome() {
  const s = state.status;
  const unsupported = s.model !== null && s.model !== 0x30;
  let text = statusText(s);
  let warn = false;
  if (unsupported) {
    text = `Modelo de mouse 0x${s.model.toString(16)} não suportado — bateria é mostrada, ajustes não são enviados`;
    warn = true;
  } else if (note?.err) {
    text += ` — ${note.text}`;
    warn = true;
  } else if (pushing) {
    text += " — enviando…";
  } else if (state.pending.length) {
    const what = state.pending.map((p) => sectionNames[p] ?? p).join(", ");
    text += ` — aguardando o mouse para enviar: ${what}`;
  }
  $("#conn").textContent = text;
  $("#conn").classList.toggle("warn", warn);

  // What is on the mouse right now. The stage, polling and light mode come
  // from the mouse when it changes them itself, otherwise from what was last sent.
  const p = active();
  const stage = p.sensor.stages[p.sensor.current_stage - 1];
  const bits = [`DPI ${stage.dpi} (estágio ${p.sensor.current_stage})`, `${p.polling.replace("Hz", "")} Hz`, `luz: ${LIGHT_MODES[p.power_light.light_mode] ?? "?"}`];
  $("#live").textContent = state.store.last_applied ? bits.join(" · ") : "";

  const sel = $("#profile");
  sel.replaceChildren(...state.store.profiles.map((p) => el("option", { value: p.name, textContent: p.name, selected: p.name === state.store.active })));
  $("#delProfile").disabled = state.store.profiles.length < 2;
}

function renderTab() {
  const unsupported = state.status.model !== null && state.status.model !== 0x30;
  document.querySelectorAll("nav button").forEach((b) => b.classList.toggle("on", b.dataset.tab === tab));
  const body = { sensor: sensorTab, buttons: buttonsTab, scripts: scriptsTab, macros: macrosTab, light: lightTab, power: powerTab }[tab]();
  body.classList.toggle("disabled-all", unsupported);
  $("#tab").replaceChildren(body);
}

// Every edit goes to the mouse by itself, shortly after the last change.
function changed() {
  note = null;
  render();
  clearTimeout(pushTimer);
  pushTimer = setTimeout(push, 500);
}

async function push() {
  pushTimer = null;
  if (pushing) return (pushTimer = setTimeout(push, 200));
  pushing = true;
  renderChrome();
  const sentCopy = structuredClone(draft);
  try {
    await invoke("update_profile", { profile: sentCopy });
    note = null;
  } catch (e) {
    // "asleep"/"no_receiver": the edit is saved and goes out when the mouse is back;
    // the header shows it as waiting. Anything else is worth showing.
    note = ["asleep", "no_receiver"].includes(e?.kind) ? null : { text: e?.message ?? String(e), err: true };
  }
  pushing = false;
  await refresh();
}

function num(value, min, max, step, onchange) {
  const i = el("input", { type: "number", value, min, max, step });
  i.onchange = () => {
    let v = Math.round(Number(i.value) / step) * step;
    v = Math.min(max, Math.max(min, isNaN(v) ? min : v));
    onchange(v); changed();
  };
  return i;
}
function check(value, onchange) {
  const i = el("input", { type: "checkbox", checked: value });
  i.onchange = () => { onchange(i.checked); changed(); };
  return i;
}
function select(options, value, onchange) {
  const s = el("select");
  options.forEach(([v, label]) => s.append(el("option", { value: JSON.stringify(v), textContent: label, selected: same(v, value) })));
  s.onchange = () => { onchange(JSON.parse(s.value)); changed(); };
  return s;
}
const row = (name, ...kids) => el("div", { className: "row" }, el("span", { className: "name", textContent: name }), ...kids);

function sensorTab() {
  const d = draft.sensor;
  const table = el("table", {}, el("tr", {}, ...["Estágio", "Ativo", "DPI", "Cor", "Atual"].map((t) => el("th", { textContent: t }))));
  d.stages.forEach((st, i) => {
    const cur = el("input", { type: "radio", name: "cur", checked: d.current_stage === i + 1, disabled: !st.enabled });
    cur.onchange = () => { d.current_stage = i + 1; changed(); };
    const color = el("input", { type: "color", value: hex(st.color) });
    color.onchange = () => { st.color = rgb(color.value); changed(); };
    table.append(el("tr", { className: st.enabled ? "" : "off" },
      el("td", { textContent: `${i + 1}` }),
      el("td", {}, check(st.enabled, (v) => {
        // At least one stage must stay on.
        if (!v && d.stages.filter((x) => x.enabled).length === 1) return;
        st.enabled = v;
        if (!d.stages[d.current_stage - 1].enabled) d.current_stage = d.stages.findIndex((x) => x.enabled) + 1;
      })),
      el("td", {}, num(st.dpi, 50, 26000, 50, (v) => (st.dpi = v))),
      el("td", {}, color),
      el("td", {}, cur)));
  });
  const live = "";
  return el("div", {},
    el("h2", { textContent: "Estágios de DPI" }), table, live,
    el("h2", { textContent: "Sensor" }),
    row("Polling rate", select(POLLING, draft.polling, (v) => (draft.polling = v)),
      el("span", { className: "muted", textContent: ["Hz2000", "Hz4000", "Hz8000"].includes(draft.polling) ? "acima de 1000 Hz o Mac não consegue medir; enviado como o driver envia" : "" })),
    row("Distância de levantamento", select([["Mm1", "1 mm"], ["Mm2", "2 mm"]], d.lod, (v) => (d.lod = v)), unverified("lod")),
    row("Ripple control", check(d.ripple, (v) => (d.ripple = v)), unverified("ripple")),
    row("Angle snapping", check(d.angle_snap, (v) => (d.angle_snap = v)), unverified("angle_snap")),
    row("Motion sync", check(d.motion_sync, (v) => (d.motion_sync = v)), unverified("motion_sync")));
}

function actionSelect(cur, onchange) {
  const sel = el("select");
  let found = false;
  for (const [group, items] of ACTION_GROUPS) {
    const og = el("optgroup", { label: group });
    for (const [label, v] of items) {
      const hit = same(v, cur);
      found ||= hit;
      og.append(el("option", { value: JSON.stringify(v), textContent: label, selected: hit }));
    }
    sel.append(og);
  }
  if (!found) {
    const label = cur?.Key ? `Atalho: ${describeKey(cur.Key)}` : "Outro (mantido)";
    sel.prepend(el("option", { value: JSON.stringify(cur), textContent: label, selected: true }));
  }
  sel.onchange = () => { onchange(JSON.parse(sel.value)); changed(); };
  return sel;
}

// "Gravar atalho": the next key combination pressed becomes the button's function.
function recordButton(onrecorded) {
  const b = el("button", { textContent: "Gravar atalho…" });
  b.onclick = () => {
    b.textContent = "Pressione o atalho (Esc cancela)";
    const onkey = (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (["Meta", "Control", "Shift", "Alt"].includes(e.key)) return; // wait for the real key
      window.removeEventListener("keydown", onkey, true);
      const usage = HID_USAGE[e.code];
      if (e.code === "Escape" || usage === undefined) return render();
      const modifiers = (e.ctrlKey ? 1 : 0) | (e.shiftKey ? 2 : 0) | (e.altKey ? 4 : 0) | (e.metaKey ? 8 : 0);
      onrecorded({ Key: { modifiers, usage } });
      changed();
    };
    window.addEventListener("keydown", onkey, true);
  };
  return b;
}

function buttonsTab() {
  const box = el("div", {}, el("h2", { textContent: "Função de cada botão (gravada no mouse)" }));
  state.slot_names.forEach((name, slot) => {
    if (!name || slot >= 16) return;
    const script = draft.scripts.find((s) => s.slot === slot);
    const kids = [];
    if (slot === 0) kids.push(el("span", { className: "muted", textContent: "Clique esquerdo (fixo, por segurança)" }));
    else if (script) kids.push(el("span", { className: "muted", textContent: `Script: ${script.label || script.command} — altere na aba Scripts` }));
    else if (draft.buttons[slot] === "Macro") kids.push(el("span", { className: "muted", textContent: `Macro: ${draft.macros.find((m) => m.slot === slot)?.name ?? ""} — altere na aba Macros` }));
    else {
      kids.push(actionSelect(draft.buttons[slot], (v) => (draft.buttons[slot] = v)));
      kids.push(recordButton((v) => (draft.buttons[slot] = v)));
    }
    box.append(row(name, ...kids));
  });
  return box;
}

function scriptsTab() {
  const box = el("div", {},
    el("h2", { textContent: "Botão → script" }),
    el("div", { className: "muted", textContent: "O botão passa a rodar o comando no Mac e perde a função original enquanto estiver atribuído. Só funciona com este app aberto." }));
  for (const slot of SCRIPT_SLOTS) {
    const b = active().scripts.find((s) => s.slot === slot);
    const typed = (scriptDraft[slot] ??= { label: b?.label ?? "", command: b?.command ?? "" });
    const label = el("input", { type: "text", placeholder: "Nome (ex.: Calculadora)", value: typed.label });
    const cmd = el("input", { type: "text", placeholder: "Comando (ex.: open -a Calculator)", value: typed.command });
    label.oninput = () => (typed.label = label.value);
    cmd.oninput = () => (typed.command = cmd.value);
    const save = el("button", { textContent: b ? "Salvar" : "Atribuir" });
    save.onclick = () => {
      delete scriptDraft[slot];
      act(() => invoke("bind_script", { profile: state.store.active, slot, command: cmd.value, label: label.value }), "Script atribuído.");
    };
    const kids = [el("div", { className: "grow" }, label), el("div", { className: "grow" }, cmd), save];
    if (b) {
      const rm = el("button", { className: "danger", textContent: "Remover" });
      rm.onclick = () => {
        delete scriptDraft[slot];
        act(() => invoke("unbind_script", { profile: state.store.active, slot }), "Script removido; botão voltou à função original.");
      };
      kids.push(rm);
    }
    box.append(row(state.slot_names[slot], ...kids));
  }
  const log = el("div", { id: "log" });
  box.append(el("h2", { textContent: "Últimas execuções" }), log);
  invoke("script_log").then((runs) => {
    if (!runs.length) return log.append(el("div", { className: "muted", textContent: "Nenhuma ainda." }));
    for (const r of runs.slice(0, 10)) {
      const when = new Date(r.started_ms).toLocaleTimeString();
      const res = r.exit === null ? "rodando…" : r.exit === 0 ? "ok" : `saiu com código ${r.exit}`;
      log.append(el("div", { textContent: `${when} — ${r.label}: ${res}` }));
      if (r.output) log.append(el("pre", { textContent: r.output + (r.truncated ? "\n[saída cortada em 64 KiB]" : "") }));
    }
  });
  return box;
}

// ---- Macros: stored in the mouse, one per button; they work without the app.
const FACTORY_ACTION = { 3: "DpiCycle", 6: "Forward", 7: "Back" };
const MACRO_MAX_STEPS = 46;
const MOD_USAGE = { ControlLeft: 0xe0, ShiftLeft: 0xe1, AltLeft: 0xe2, MetaLeft: 0xe3, ControlRight: 0xe4, ShiftRight: 0xe5, AltRight: 0xe6, MetaRight: 0xe7 };
const MOD_NAME = { 0xe0: "⌃", 0xe1: "⇧", 0xe2: "⌥", 0xe3: "⌘", 0xe4: "⌃ dir.", 0xe5: "⇧ dir.", 0xe6: "⌥ dir.", 0xe7: "⌘ dir.",
  0xf1: "Clique esquerdo", 0xf2: "Clique direito", 0xf3: "Clique do meio", 0xf4: "Avançar", 0xf5: "Voltar" };
const codeName = (c) => MOD_NAME[c] ?? USAGE_NAME[c] ?? `tecla 0x${c.toString(16)}`;
// Nearest delay the firmware can store: 10, 30, 50 … 1270 ms.
function snapDelay(ms) {
  const tens = Math.min(127, Math.max(1, Math.round(ms / 10)));
  const odd = tens % 2 === 1 ? tens : ms >= tens * 10 ? tens + 1 : tens - 1;
  return Math.min(127, Math.max(1, odd)) * 10;
}
let recording = null; // { slot, steps, last, down:Set }

function startRecording(slot) {
  recording = { slot, steps: [], last: performance.now(), down: new Set() };
  const onkey = (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.repeat || !recording) return;
    const code = MOD_USAGE[e.code] ?? HID_USAGE[e.code];
    if (code === undefined) return;
    const isDown = e.type === "keydown";
    if (isDown === recording.down.has(code)) return; // ignore stray repeats/releases
    if (isDown && recording.steps.length >= MACRO_MAX_STEPS - recording.down.size - 1) return; // leave room for releases
    const now = performance.now();
    const prev = recording.steps[recording.steps.length - 1];
    if (prev) prev.delay_ms = snapDelay(now - recording.last);
    recording.last = now;
    isDown ? recording.down.add(code) : recording.down.delete(code);
    recording.steps.push({ down: isDown, code, delay_ms: 10 });
    renderTab();
  };
  recording.stop = () => {
    window.removeEventListener("keydown", onkey, true);
    window.removeEventListener("keyup", onkey, true);
  };
  window.addEventListener("keydown", onkey, true);
  window.addEventListener("keyup", onkey, true);
  renderTab();
}

function stopRecording(keep) {
  const r = recording;
  recording = null;
  r.stop();
  // Never leave a key down: release whatever is still held.
  for (const code of r.down) r.steps.push({ down: false, code, delay_ms: 10 });
  if (keep && r.steps.length) {
    const existing = draft.macros.find((m) => m.slot === r.slot);
    if (existing) existing.definition.steps = r.steps;
    else draft.macros.push({ slot: r.slot, name: "Macro", definition: { steps: r.steps, repeat: { Times: 1 } } });
    draft.buttons[r.slot] = "Macro";
    return changed();
  }
  renderTab();
}

function macrosTab() {
  const box = el("div", {},
    el("h2", { textContent: "Macros gravadas no mouse" }),
    el("div", { className: "muted", textContent: "Sequência de teclas num botão, guardada no próprio mouse — funciona sem este app. Até 46 ações; pausas de 10 a 1270 ms. O botão perde a função original enquanto tiver macro." }));
  for (const slot of SCRIPT_SLOTS) {
    const name = state.slot_names[slot];
    box.append(el("h2", { textContent: name }));
    if (draft.scripts.some((s) => s.slot === slot)) {
      box.append(el("div", { className: "muted", textContent: "Este botão tem um script. Remova o script para usar macro." }));
      continue;
    }
    if (recording?.slot === slot) {
      const stop = el("button", { className: "primary", textContent: "Parar e salvar" });
      stop.onclick = () => stopRecording(true);
      const cancel = el("button", { textContent: "Cancelar" });
      cancel.onclick = () => stopRecording(false);
      box.append(row("Gravando…", stop, cancel),
        el("div", { textContent: recording.steps.map((s) => (s.down ? "↓" : "↑") + codeName(s.code)).join("  ") || "pressione as teclas da macro" }));
      continue;
    }
    const m = draft.macros.find((x) => x.slot === slot);
    const rec = el("button", { textContent: m ? "Regravar…" : "Gravar macro…", disabled: !!recording });
    rec.onclick = () => startRecording(slot);
    if (!m) { box.append(row("", rec)); continue; }

    const nameInput = el("input", { type: "text", value: m.name });
    nameInput.onchange = () => { m.name = nameInput.value || "Macro"; changed(); };
    const rm = el("button", { className: "danger", textContent: "Remover macro" });
    rm.onclick = () => {
      draft.macros = draft.macros.filter((x) => x.slot !== slot);
      draft.buttons[slot] = FACTORY_ACTION[slot] ?? "Disabled";
      changed();
    };
    box.append(row("Nome", el("div", { className: "grow" }, nameInput), rec, rm));

    const rep = m.definition.repeat;
    const mode = typeof rep === "string" ? rep : "Times";
    const modeSel = select([["Times", "Tocar N vezes"], ["WhileHeld", "Repetir enquanto segura o botão"], ["UntilKeyPressed", "Repetir até apertar qualquer tecla"]], mode,
      (v) => (m.definition.repeat = v === "Times" ? { Times: 1 } : v));
    const kids = [modeSel];
    if (mode === "Times") kids.push(num(rep.Times, 1, 255, 1, (v) => (m.definition.repeat = { Times: v })), "vez(es)");
    box.append(row("Repetição", ...kids));

    const table = el("table", {}, el("tr", {}, ...["#", "Ação", "Pausa depois (ms)"].map((t) => el("th", { textContent: t }))));
    m.definition.steps.forEach((st, i) => {
      const d = el("input", { type: "number", value: st.delay_ms, min: 10, max: 1270, step: 20 });
      d.onchange = () => { st.delay_ms = snapDelay(Number(d.value) || 10); changed(); };
      table.append(el("tr", {},
        el("td", { textContent: `${i + 1}` }),
        el("td", { textContent: `${st.down ? "↓ pressiona" : "↑ solta"} ${codeName(st.code)}` }),
        el("td", {}, d)));
    });
    box.append(table);
  }
  return box;
}

function lightTab() {
  const d = draft.power_light;
  const color = el("input", { type: "color", value: hex(d.color), disabled: draft.light_follows_dpi });
  color.onchange = () => { d.color = rgb(color.value); changed(); };
  return el("div", {},
    el("h2", { textContent: "Luz" }),
    row("Modo", select(LIGHT_MODES.map((l, i) => [i, l]), d.light_mode, (v) => (d.light_mode = v))),
    row("Cor", color, el("label", {}, check(draft.light_follows_dpi, (v) => (draft.light_follows_dpi = v)), " acompanhar a cor do estágio de DPI atual")),
    row("Brilho (1–8)", num(d.brightness, 1, 8, 1, (v) => (d.brightness = v)), el("span", { className: "muted", textContent: "vale para os modos Estática e Modo 9" })),
    row("Velocidade (4–8)", num(d.breath_speed, 4, 8, 1, (v) => (d.breath_speed = v))));
}

function powerTab() {
  const d = draft.power_light;
  const st = state.store.settings;
  const setting = (patch) => act(() => invoke("update_settings", { settings: { ...st, ...patch } }));
  const low = el("input", { type: "number", min: 1, max: 100, value: st.low_battery_pct });
  low.onchange = () => setting({ low_battery_pct: Math.min(100, Math.max(1, Number(low.value) || 20)) });
  const auto = el("input", { type: "checkbox", checked: st.autostart });
  auto.onchange = () => setting({ autostart: auto.checked });
  const reset = el("button", { className: "danger", textContent: "Restaurar padrão de fábrica…" });
  reset.onclick = () => {
    if (confirm("Restaurar o mouse para os padrões de fábrica? DPI, botões, luz e energia do mouse serão substituídos. Seus perfis no app não são apagados.")) {
      act(() => invoke("factory_reset"), "Mouse restaurado para os padrões de fábrica.");
    }
  };
  return el("div", {},
    el("h2", { textContent: "Energia do mouse" }),
    row("Dormir após (min)", num(d.sleep_min, 1, 20, 1, (v) => (d.sleep_min = v)), unverified("sleep_min")),
    row("Debounce (ms)", num(d.debounce_ms, 2, 50, 2, (v) => (d.debounce_ms = v)), unverified("debounce_ms")),
    el("h2", { textContent: "App" }),
    row("Avisar bateria abaixo de (%)", low),
    row("Abrir ao iniciar sessão", auto),
    el("h2", { textContent: "Recuperação" }),
    row("", reset));
}

// Run a command, show a failure in the header, reload state.
async function act(fn) {
  try {
    await fn();
    note = null;
  } catch (e) {
    const text = Array.isArray(e) ? e.join("; ") : e?.message ?? String(e);
    note = ["asleep", "no_receiver"].includes(e?.kind) ? null : { text, err: true };
  }
  await refresh();
}

$("#profile").onchange = (e) => {
  scriptDraft = {};
  const name = e.target.value;
  act(() => invoke("set_active_profile", { name }));
};
$("#newProfile").onclick = () => {
  const name = prompt("Nome do novo perfil:");
  if (name) act(() => invoke("create_profile", { name }), `Perfil “${name.trim()}” criado.`);
};
$("#delProfile").onclick = () => {
  const others = state.store.profiles.filter((p) => p.name !== state.store.active).map((p) => p.name);
  const name = prompt(`Apagar qual perfil? (${others.join(", ")})`);
  if (name) act(() => invoke("delete_profile", { name }), `Perfil “${name}” apagado.`);
};
document.querySelectorAll("nav button").forEach((b) => (b.onclick = () => { tab = b.dataset.tab; render(); }));

listen("status", (e) => {
  if (!state) return;
  const stageChanged = state.status.dpi_stage !== e.payload.dpi_stage;
  const modelChanged = state.status.model !== e.payload.model;
  state.status = e.payload;
  renderChrome();
  // Only the Sensor tab shows live status, and it holds no free-text fields.
  if (modelChanged || (stageChanged && tab === "sensor")) renderTab();
});
listen("store-changed", () => refresh());
listen("script-run", () => { if (tab === "scripts") setTimeout(renderTab, 600); });
refresh();
