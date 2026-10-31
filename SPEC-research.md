# SPEC: `wisply research` (web research without an AI model)

Decided with QDHShamiro on 2026-10-31 (grill-me). Wisply itself calls no language model and no embedding
model here: everything is search, fetching and arithmetic. Runs while a game is running.

## Command

```
wisply research "query" ["query 2" ...] [--quick | --deep] [--site host] [--fresh] [--full]
```

- Several queries in one call (Claude passes a German and an English wording); all are searched at once
  and the results are merged.
- Levels: `--quick` 1 round, default **2 rounds**, `--deep` 3 rounds with more pages.
  | level | rounds | pages read round 1 | pages per later round | time budget |
  |---|---|---|---|---|
  | quick | 1 | 6 | – | ~4 s |
  | default | 2 | 8 | 5 | ~10 s |
  | deep | 3 | 10 | 6 | ~25 s |
- `--site github.com` limits every engine to a site. `--fresh` skips the stored report. `--full` prints the
  full extracted text path list instead of only the report (the texts always go to a file in `out/`).

## Output (a short report, ~1,500 tokens)

```
research "folia scheduler" + "folia scheduler library paper" (2 rounds, 13 pages, 7.4 s, 6 engines)
[1] FoliaLib - GitHub · github.com/TechnicallyCoded/FoliaLib · 2026-08 · 4 engines
    > FoliaLib wraps the global, region and entity schedulers ... (best 2-3 passages, ~300 chars each)
[2] ...
said by several sources:
- FoliaLib (5 sources), MorePaperLib (3), "RegionScheduler" (4), 1.21.11 (3)
next: wisply read <url> for a whole page · full texts: ~/.wisply/out/....md
saved: vault Research/2026-10-31 folia-scheduler.md (#15601)
```

## Pipeline

1. **Search** (parallel threads, 4 s timeout each, the shared `read` HTTP agent):
   DuckDuckGo HTML, Bing, Brave Search, Mojeek (result pages parsed), Wikipedia search API (de + en),
   GitHub search API, Stack Exchange API, Reddit search JSON. Each engine is a small parser with a fixture test.
   Base URLs can be overridden for tests (`WISPLY_RESEARCH_BASE`).
2. **Blocked engines**: a captcha, 403/429 or a page without results marks the engine as blocked for 30 min
   (`research/engines.json`). If every page engine is blocked, the search runs in the user's browser
   (the existing `search` path, Google), in the background without focus.
3. **Merge**: normalize URLs (scheme, `www.`, trailing slash, tracking params), Reciprocal Rank Fusion (k = 60)
   over all engines and queries, at most 2 results per domain, a domain trust prior (official docs, GitHub,
   Wikipedia, Stack Overflow up; a small list of content farms down, `research/domains.txt` editable).
4. **Read**: the top pages through the existing `read` pipeline (HTML → Markdown, PDF, gated pages skipped),
   in parallel, 6 s per page.
5. **Passages**: split each page at headings and paragraphs into ~400-800 character passages; BM25 against the
   union of query terms (stopwords de + en), plus boosts for phrase hits, hits in headings, code blocks when the
   query looks technical, and page score from step 3. Top 3 passages per page, then the best pages overall.
6. **Next round** (default and deep): pseudo-relevance feedback without a model: the terms that are much more
   frequent in the top passages than across all passages (tf-idf against the round's pool) extend each query by
   2-3 terms; links that two or more read pages point to are read too. Pages already read are skipped.
7. **Said by several sources**: names, versions, numbers with units and quoted terms that appear in passages of
   two or more different domains, counted per domain.
8. **Date**: `article:published_time`, JSON-LD `datePublished`, `<time datetime>`, or a year in the URL. Newer pages
   get a small boost when the query has a year or a version.

## Storage

- Every report goes to the vault as `Research/<date> <slug>.md` (sources, passages, the queries) and into the
  memory as a note (kind `research`), so `wisply recall` finds it later.
- Same queries (normalized, order-free) on the same day → the stored report comes back at once (`--fresh` skips).
- The full extracted texts go to `out/` like other big outputs (24 h).

## Claude

- The wisply skill: for research use `wisply research` first, with a German and an English wording when the
  question is German; built-in WebSearch only when Wisply finds nothing.
- MCP: the same command line through the `wisply` tool.

## Privacy and docs

- CODE_SIGNING.md privacy policy and SECURITY.md: research sends the search words to the engines listed above
  and loads the result pages, like a browser would.
- README: one row in "What it does", the command in Commands, a line in the skill.

## Tests

- Unit: each engine parser on a saved result page, URL normalization, RRF, BM25, query expansion, the
  several-sources list, date extraction, blocked detection.
- Integration (`tests/research.rs`): a local HTTP server plays all engines and pages; checks the report, the
  2-round flow, blocked engines, the vault file and the same-day cache. The browser fallback (Google in the
  user's browser when every search page is blocked) is checked by hand, it needs a real browser.

## Not in scope

- No language model, no embeddings, no translation.
- No crawling beyond the rounds above, no login-only pages (those stay with `open`).
