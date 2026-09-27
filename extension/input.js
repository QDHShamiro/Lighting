import { attach, send } from "./cdp.js";

const MODS = { alt: 1, option: 1, ctrl: 2, control: 2, strg: 2, meta: 4, cmd: 4, win: 4, super: 4, shift: 8, umschalt: 8, altgr: 3 };
const MOD_KEYS = { 1: ["Alt", "AltLeft", 18], 2: ["Control", "ControlLeft", 17], 4: ["Meta", "MetaLeft", 91], 8: ["Shift", "ShiftLeft", 16] };
const NAMED = {
  enter: ["Enter", "Enter", 13, "\r"],
  return: ["Enter", "Enter", 13, "\r"],
  tab: ["Tab", "Tab", 9],
  escape: ["Escape", "Escape", 27],
  esc: ["Escape", "Escape", 27],
  backspace: ["Backspace", "Backspace", 8],
  delete: ["Delete", "Delete", 46],
  del: ["Delete", "Delete", 46],
  space: [" ", "Space", 32, " "],
  up: ["ArrowUp", "ArrowUp", 38],
  down: ["ArrowDown", "ArrowDown", 40],
  left: ["ArrowLeft", "ArrowLeft", 37],
  right: ["ArrowRight", "ArrowRight", 39],
  arrowup: ["ArrowUp", "ArrowUp", 38],
  arrowdown: ["ArrowDown", "ArrowDown", 40],
  arrowleft: ["ArrowLeft", "ArrowLeft", 37],
  arrowright: ["ArrowRight", "ArrowRight", 39],
  home: ["Home", "Home", 36],
  end: ["End", "End", 35],
  pageup: ["PageUp", "PageUp", 33],
  pagedown: ["PageDown", "PageDown", 34],
  pgup: ["PageUp", "PageUp", 33],
  pgdn: ["PageDown", "PageDown", 34],
  insert: ["Insert", "Insert", 45],
  ins: ["Insert", "Insert", 45],
  bksp: ["Backspace", "Backspace", 8],
  entf: ["Delete", "Delete", 46],
  pos1: ["Home", "Home", 36],
  plus: ["+", "Equal", 187, "+"],
  minus: ["-", "Minus", 189, "-"],
  comma: [",", "Comma", 188, ","],
  period: [".", "Period", 190, "."],
};

function keyDef(name) {
  const low = name.toLowerCase();
  if (NAMED[low]) return NAMED[low];
  const f = /^f([1-9]|1[0-9]|2[0-4])$/.exec(low);
  if (f) return ["F" + f[1], "F" + f[1], 111 + Number(f[1])];
  if (name.length === 1) {
    const c = name;
    if (/[a-z]/i.test(c)) return [c, "Key" + c.toUpperCase(), c.toUpperCase().charCodeAt(0), c];
    if (/[0-9]/.test(c)) return [c, "Digit" + c, c.charCodeAt(0), c];
    return [c, "", 0, c];
  }
  throw new Error("unknown key '" + name + "'");
}

export function parseCombo(combo) {
  const c = combo.replace(/\s+/g, "");
  const parts = c === "+" || c.endsWith("++") ? c.slice(0, -2).split("+").filter(Boolean).concat(["+"]) : c.split("+").filter(Boolean);
  let mods = 0;
  const keys = [];
  for (const p of parts) {
    const m = MODS[p.toLowerCase()];
    if (m && parts.length > 1) mods |= m;
    else keys.push(p);
  }
  if (!keys.length) {
    const last = parts[parts.length - 1];
    if (last) keys.push(last);
  }
  return { mods, key: keys[keys.length - 1] };
}

export async function press(tabId, combo) {
  await attach(tabId);
  const { mods, key } = parseCombo(combo);
  const held = [];
  for (const bit of [2, 1, 4, 8]) {
    if (mods & bit) {
      held.push(bit);
      const [k, code, vk] = MOD_KEYS[bit];
      const now = held.reduce((a, b) => a | b, 0);
      await send(tabId, "Input.dispatchKeyEvent", { type: "rawKeyDown", key: k, code, windowsVirtualKeyCode: vk, modifiers: now });
    }
  }
  const [k, code, vk, text] = keyDef(key);
  const plain = text !== undefined && !(mods & (1 | 2 | 4));
  const ev = { key: k, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers: mods };
  if (plain) await send(tabId, "Input.dispatchKeyEvent", Object.assign({ type: "keyDown", text, unmodifiedText: text }, ev));
  else await send(tabId, "Input.dispatchKeyEvent", Object.assign({ type: "rawKeyDown" }, ev));
  await send(tabId, "Input.dispatchKeyEvent", Object.assign({ type: "keyUp" }, ev));
  let now = mods;
  for (const bit of held.reverse()) {
    now &= ~bit;
    const [mk, mcode, mvk] = MOD_KEYS[bit];
    await send(tabId, "Input.dispatchKeyEvent", { type: "keyUp", key: mk, code: mcode, windowsVirtualKeyCode: mvk, modifiers: now });
  }
}

export const timings = [];

export async function click(tabId, x, y, opts) {
  timings.length = 0;
  let t = Date.now();
  const lap = (n) => {
    const now = Date.now();
    timings.push(n + " " + (now - t));
    t = now;
  };
  await attach(tabId);
  lap("attach");
  const button = (opts && opts.button) || "left";
  const count = (opts && opts.count) || 1;
  const buttons = { left: 1, right: 2, middle: 4 }[button] || 1;
  send(tabId, "Input.dispatchMouseEvent", { type: "mouseMoved", x, y }).catch(() => {});
  for (let i = 1; i <= count; i++) {
    await send(tabId, "Input.dispatchMouseEvent", { type: "mousePressed", x, y, button, buttons, clickCount: i });
    lap("down");
    await send(tabId, "Input.dispatchMouseEvent", { type: "mouseReleased", x, y, button, buttons: 0, clickCount: i });
    lap("up");
  }
}

export async function move(tabId, x, y) {
  await attach(tabId);
  send(tabId, "Input.dispatchMouseEvent", { type: "mouseMoved", x, y }).catch(() => {});
  await new Promise((r) => setTimeout(r, 80));
}

export async function wheel(tabId, x, y, dy) {
  await attach(tabId);
  await send(tabId, "Input.dispatchMouseEvent", { type: "mouseWheel", x, y, deltaX: 0, deltaY: dy });
}

export async function drag(tabId, a, b) {
  await attach(tabId);
  await send(tabId, "Input.dispatchMouseEvent", { type: "mousePressed", x: a.x, y: a.y, button: "left", clickCount: 1 });
  const steps = 10;
  for (let i = 1; i <= steps; i++) {
    const x = a.x + ((b.x - a.x) * i) / steps;
    const y = a.y + ((b.y - a.y) * i) / steps;
    send(tabId, "Input.dispatchMouseEvent", { type: "mouseMoved", x, y, button: "left", buttons: 1 }).catch(() => {});
    await new Promise((r) => setTimeout(r, 16));
  }
  await send(tabId, "Input.dispatchMouseEvent", { type: "mouseReleased", x: b.x, y: b.y, button: "left", clickCount: 1 });
}

export async function insertText(tabId, text) {
  await attach(tabId);
  await send(tabId, "Input.insertText", { text });
}
