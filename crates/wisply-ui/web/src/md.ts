import { Marked, type Tokens } from "marked";

const ESC: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };

export function escape(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ESC[c]);
}

type Wiki = { type: "wikilink"; raw: string; target: string; label: string };

const WIKI = /^\[\[([^\]|#\n]+)(?:#[^\]|\n]*)?(?:\|([^\]\n]+))?\]\]/;

function safeHref(href: string): string | null {
  if (/^https?:\/\//i.test(href) || href.startsWith("#")) return href;
  return null;
}

const marked = new Marked({
  gfm: true,
  breaks: false,
  extensions: [
    {
      name: "wikilink",
      level: "inline",
      start: (src: string) => src.indexOf("[["),
      tokenizer(src: string) {
        const m = WIKI.exec(src);
        if (!m) return undefined;
        const target = m[1].trim();
        return { type: "wikilink", raw: m[0], target, label: (m[2] ?? m[1]).trim() } satisfies Wiki;
      },
      renderer(token) {
        const t = token as unknown as Wiki;
        return `<a class="wiki" data-title="${escape(t.target)}" href="#/t/${encodeURIComponent(t.target)}">${escape(t.label)}</a>`;
      },
    },
  ],
  renderer: {
    html(token: Tokens.HTML | Tokens.Tag) {
      return escape(token.text);
    },
    link(token: Tokens.Link) {
      const text = this.parser.parseInline(token.tokens);
      const href = safeHref(token.href);
      if (!href) return text;
      const outside = !href.startsWith("#");
      return `<a href="${escape(href)}"${outside ? ' target="_blank" rel="noreferrer"' : ""}>${text}</a>`;
    },
    image(token: Tokens.Image) {
      return escape(token.text || token.href);
    },
  },
});

export function render(body: string): string {
  const text = body.replace(/^---\n[\s\S]*?\n---\n/, "");
  return marked.parse(text, { async: false }) as string;
}
