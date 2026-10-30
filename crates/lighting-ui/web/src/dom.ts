type Child = Node | string | number | null | undefined | false;
type Attrs = Record<string, string | number | boolean | ((e: Event) => void) | undefined>;

export function h<K extends keyof HTMLElementTagNameMap>(tag: K, attrs: Attrs = {}, ...kids: (Child | Child[])[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === false) continue;
    if (typeof v === "function") el.addEventListener(k.replace(/^on/, ""), v as EventListener);
    else if (v === true) el.setAttribute(k, "");
    else el.setAttribute(k, String(v));
  }
  for (const kid of kids.flat()) {
    if (kid === null || kid === undefined || kid === false) continue;
    el.append(kid instanceof Node ? kid : String(kid));
  }
  return el;
}

export function mount(parent: HTMLElement, ...kids: Child[]): void {
  parent.replaceChildren(...kids.filter((k): k is Node | string => k !== null && k !== undefined && k !== false).map((k) => (typeof k === "number" ? String(k) : k)));
}

export function ago(t: number): number {
  return Math.max(0, Date.now() / 1000 - t);
}

export function go(hash: string): void {
  if (location.hash !== hash) location.hash = hash;
}

export function enc(s: string): string {
  return encodeURIComponent(s);
}

export function debounce<A extends unknown[]>(f: (...a: A) => void, ms: number): (...a: A) => void {
  let timer = 0;
  return (...a: A) => {
    clearTimeout(timer);
    timer = window.setTimeout(() => f(...a), ms);
  };
}
