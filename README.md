# kips.dev and kccs.dev

Two sober, fast references for the Kaspa proposal repositories:

- **[kips.dev](https://kips.dev)** presents Kaspa Improvement Proposals.
- **[kccs.dev](https://kccs.dev)** presents Kaspa Calls for Conventions.

Canonical documents get short, shareable URLs, a searchable index, readable proposal pages, status and category filters, companion-document support, and links back to their source. The two sites share one design and explicitly link to one another.

## Architecture

The build stays deliberately small:

1. A Rust importer updates managed checkouts of the canonical KIP and KCC repositories, then reads their fenced Markdown metadata.
2. It prepares two Zola 0.23.4 sites under `.generated/`, rewrites proposal and companion links, and preserves auxiliary files such as test vectors.
3. Zola produces plain static files and JSON search input in `dist/kips` and `dist/kccs`.
4. Tinysearch 0.11.0 compiles each full-text index into a small Rust/WebAssembly module; search terms are subtly highlighted in results and in opened documents.

The managed source checkouts, generated sites, and final output are ignored by Git. The only browser-side code handles search, filters, theme selection, and copying a document URL.

An automated daily run fetches the canonical repositories, rebuilds both sites, and deploys updates when their source revisions change.

## Local development

Local previews require Node.js 22 and Rust 1.96.0. Install the pinned site tools once:

```bash
cargo install --locked --git https://github.com/getzola/zola --tag v0.23.4
cargo install tinysearch --version 0.11.0 --locked --features bin
```

Then use the matching npm command:

```bash
npm run preview:kips  # http://127.0.0.1:1111
npm run preview:kccs  # http://127.0.0.1:1112
```

Each command regenerates the sites and search index before starting a local preview. Restart it after changing the templates, static files, importer, or proposal content.

## Content ownership

Proposal content remains owned and licensed by its respective authors and source repositories. The Kaspa symbol is the asset served by [kaspa.org](https://kaspa.org/); it is used sparingly for project identification.

## Acknowledgements

The reading experience is inspired by [bips.dev](https://bips.dev).
