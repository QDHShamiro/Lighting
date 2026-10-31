export type Done = { code: number; out: string };

declare global {
  interface Window {
    lt: { done(id: number, code: number, out: string): void };
    ipc: { postMessage(message: string): void };
  }
}

const waiting = new Map<number, (d: Done) => void>();
let next = 1;

window.lt = {
  done(id, code, out) {
    const resolve = waiting.get(id);
    if (resolve) {
      waiting.delete(id);
      resolve({ code, out });
    }
  },
};

export function call(argv: string[], body?: string): Promise<Done> {
  const id = next++;
  return new Promise((resolve) => {
    waiting.set(id, resolve);
    window.ipc.postMessage(JSON.stringify({ id, argv, body }));
  });
}

function clean(out: string): string {
  return out.replace(/^err: /, "").split(" -> try:")[0];
}

export async function board<T>(...args: string[]): Promise<T> {
  const r = await call(["board", ...args]);
  if (r.code !== 0) throw new Error(clean(r.out));
  return JSON.parse(r.out) as T;
}

export async function save(path: string, body: string, mtime: number): Promise<{ ok?: boolean; conflict?: boolean; mtime: number }> {
  const r = await call(["board", "save", path, "--mtime", String(mtime)], body);
  if (r.code !== 0) throw new Error(clean(r.out));
  return JSON.parse(r.out);
}

export async function plain(...argv: string[]): Promise<string> {
  const r = await call(argv);
  if (r.code !== 0) throw new Error(clean(r.out));
  return r.out;
}

export type Card = { name: string; notes: number; todos: number; sessions: number; last: number; title: string };
export type Pulse = { pid: number; up: number; working: number; queued: number; drafts: number; notes: number; problem: string };
export type Home = Pulse & { projects: Card[] };
export type NoteRow = { id: number; title: string; project: string; kind: string; source: string; updated: number; path: string };
export type Todo = { id: number; task: string; status: string; list: string; note: string; updated: number };
export type Run = { id: string; title: string; summary: string; next: string; last: number; status: string };
export type Project = { name: string; about: string; notes: NoteRow[]; todos: Todo[]; sessions: Run[] };
export type Note = NoteRow & {
  body: string;
  mtime: number;
  editable: boolean;
  links: string[];
  tags: string[];
  backlinks: { id: number; title: string }[];
};
export type Hit = NoteRow & { score: number; snippet: string };
export type GraphData = { nodes: [string, string, string, string, number][]; edges: [number, number][] };
export type LiveRow = { sid: string; cmd: string; ref: string; ok: boolean; error?: string; t: string; age: number; count: number; project?: string; title?: string };
export type Live = { sessions: LiveRow[]; working_s: number; poll_ms: number };
