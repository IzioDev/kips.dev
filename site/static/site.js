(() => {
  const root = document.documentElement;
  const themeButton = document.querySelector(".theme-toggle");

  const applyThemeLabel = () => {
    if (!themeButton) return;
    const dark = root.dataset.theme === "dark";
    themeButton.setAttribute("aria-label", dark ? "Switch to light theme" : "Switch to dark theme");
    themeButton.setAttribute("title", dark ? "Switch to light theme" : "Switch to dark theme");
  };

  themeButton?.addEventListener("click", () => {
    const next = root.dataset.theme === "dark" ? "light" : "dark";
    if (next === "dark") root.dataset.theme = "dark";
    else delete root.dataset.theme;
    try { localStorage.setItem("theme", next); } catch (_) {}
    applyThemeLabel();
  });
  applyThemeLabel();

  const copyButton = document.querySelector(".copy-link");
  copyButton?.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(location.href);
      const previous = copyButton.textContent;
      copyButton.textContent = "Copied";
      window.setTimeout(() => { copyButton.textContent = previous; }, 1600);
    } catch (_) {
      copyButton.textContent = "Copy failed";
    }
  });

  const prose = document.querySelector(".prose");
  if (prose && typeof window.renderMathInElement === "function") {
    window.renderMathInElement(prose, {
      delimiters: [
        { left: "$$", right: "$$", display: true },
        { left: "$", right: "$", display: false },
        { left: "\\[", right: "\\]", display: true },
        { left: "\\(", right: "\\)", display: false },
      ],
      throwOnError: false,
    });
  }

  const normalize = (value) => value.trim().toLocaleLowerCase();
  const highlightTerms = (query) => [...new Set(
    normalize(query).match(/[\p{L}\p{N}']{2,}/gu) || [],
  )].sort((left, right) => right.length - left.length);
  const highlightPattern = (query) => {
    const terms = highlightTerms(query);
    if (!terms.length) return null;
    const escaped = terms.map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
    return new RegExp(escaped.join("|"), "giu");
  };
  const appendHighlightedText = (element, text, query) => {
    const pattern = highlightPattern(query);
    if (!pattern) {
      element.append(document.createTextNode(text));
      return;
    }

    let cursor = 0;
    for (const match of text.matchAll(pattern)) {
      element.append(document.createTextNode(text.slice(cursor, match.index)));
      const mark = document.createElement("mark");
      mark.className = "search-highlight";
      mark.textContent = match[0];
      element.append(mark);
      cursor = match.index + match[0].length;
    }
    element.append(document.createTextNode(text.slice(cursor)));
  };
  const searchExcerpt = (text, query, maxLength = 180) => {
    const compact = text.replace(/\s+/g, " ").trim();
    if (compact.length <= maxLength) return compact;

    const normalizedText = normalize(compact);
    const matchPositions = highlightTerms(query)
      .map((term) => normalizedText.indexOf(term))
      .filter((position) => position >= 0);
    const matchPosition = matchPositions.length ? Math.min(...matchPositions) : 0;
    let start = Math.max(0, matchPosition - Math.floor(maxLength / 3));
    let end = Math.min(compact.length, start + maxLength);

    if (end - start < maxLength) start = Math.max(0, end - maxLength);
    if (start > 0) {
      const nextSpace = compact.indexOf(" ", start);
      if (nextSpace >= 0 && nextSpace < end) start = nextSpace + 1;
    }
    if (end < compact.length) {
      const previousSpace = compact.lastIndexOf(" ", end);
      if (previousSpace > start) end = previousSpace;
    }

    return `${start > 0 ? "…" : ""}${compact.slice(start, end)}${end < compact.length ? "…" : ""}`;
  };

  const highlightDocument = () => {
    const query = new URLSearchParams(location.search).get("highlight")?.trim();
    const pattern = query && highlightPattern(query);
    if (!prose || !query || !pattern) return;

    const walker = document.createTreeWalker(prose, NodeFilter.SHOW_TEXT, {
      acceptNode: (node) => {
        const parent = node.parentElement;
        if (!parent || !node.data.trim()) return NodeFilter.FILTER_REJECT;
        if (parent.closest(".katex, code, pre, script, style, mark, textarea")) {
          return NodeFilter.FILTER_REJECT;
        }
        return NodeFilter.FILTER_ACCEPT;
      },
    });
    const textNodes = [];
    while (walker.nextNode()) textNodes.push(walker.currentNode);

    let matchCount = 0;
    for (const node of textNodes) {
      if (matchCount >= 120) break;
      const fragment = document.createDocumentFragment();
      let cursor = 0;
      let found = false;
      for (const match of node.data.matchAll(pattern)) {
        if (matchCount >= 120) break;
        found = true;
        fragment.append(document.createTextNode(node.data.slice(cursor, match.index)));
        const mark = document.createElement("mark");
        mark.className = "search-highlight";
        mark.textContent = match[0];
        fragment.append(mark);
        cursor = match.index + match[0].length;
        matchCount += 1;
      }
      if (found) {
        fragment.append(document.createTextNode(node.data.slice(cursor)));
        node.replaceWith(fragment);
      }
    }
  };
  highlightDocument();

  const authorDisclosures = [...document.querySelectorAll(".author-disclosure")];
  const closeAuthorDisclosures = (exception) => {
    authorDisclosures.forEach((disclosure) => {
      if (disclosure === exception) return;
      disclosure.classList.remove("is-open");
      disclosure.querySelector(".author-trigger")?.setAttribute("aria-expanded", "false");
    });
  };

  authorDisclosures.forEach((disclosure) => {
    const trigger = disclosure.querySelector(".author-trigger");
    trigger?.addEventListener("click", () => {
      const shouldOpen = !disclosure.classList.contains("is-open");
      disclosure.classList.toggle("is-dismissed", !shouldOpen);
      closeAuthorDisclosures(disclosure);
      disclosure.classList.toggle("is-open", shouldOpen);
      trigger.setAttribute("aria-expanded", String(shouldOpen));
    });
    trigger?.addEventListener("blur", () => disclosure.classList.remove("is-dismissed"));
    disclosure.addEventListener("pointerenter", () => disclosure.classList.remove("is-dismissed"));
  });

  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    const activeDisclosure = document.activeElement?.closest?.(".author-disclosure");
    closeAuthorDisclosures();
    activeDisclosure?.classList.add("is-dismissed");
    activeDisclosure?.querySelector(".author-trigger")?.focus();
  });

  document.addEventListener("click", (event) => {
    if (!event.target.closest?.(".author-disclosure")) closeAuthorDisclosures();
  });

  const searchInput = document.getElementById("search-input");
  if (!searchInput) return;

  const rows = [...document.querySelectorAll(".proposal-row")];
  const statusFilter = document.getElementById("status-filter");
  const facetFilter = document.getElementById("facet-filter");
  const visibleCount = document.getElementById("visible-count");
  const emptyState = document.querySelector(".empty-state");
  const results = document.querySelector(".search-results");
  const resultsList = document.querySelector(".search-results-list");
  let indexPromise;
  let debounceTimer;
  let fullTextQuery = "";
  let fullTextProposalPaths = new Set();

  rows.forEach((row) => {
    const link = row.querySelector(".proposal-row-link");
    if (link) row.dataset.baseHref = link.getAttribute("href");
  });

  const proposalPath = (value) => {
    const path = new URL(value, location.origin).pathname;
    const match = path.match(/^\/(\d+)(?:\/|$)/);
    return match ? `/${match[1]}/` : "";
  };

  const filterRows = () => {
    const query = normalize(searchInput.value);
    const status = normalize(statusFilter?.value || "");
    const facet = normalize(facetFilter?.value || "");
    let count = 0;

    rows.forEach((row) => {
      const link = row.querySelector(".proposal-row-link");
      if (!link) return;
      const matchesMetadata = normalize(row.dataset.search || "").includes(query);
      const matchesContent = query.length >= 2
        && query === fullTextQuery
        && fullTextProposalPaths.has(proposalPath(link.href));
      const matches = (!query || matchesMetadata || matchesContent)
        && (!status || row.dataset.status === status)
        && (!facet || normalize(row.dataset.facets || "").split(", ").includes(facet));
      link.href = query.length >= 2 && matches
        ? resultUrl(row.dataset.baseHref, searchInput.value.trim())
        : row.dataset.baseHref;
      row.hidden = !matches;
      if (matches) count += 1;
    });

    visibleCount.textContent = String(count);
    emptyState.hidden = count !== 0;
  };

  const loadIndex = () => {
    if (!indexPromise) {
      indexPromise = import("/tinysearch.js")
        .then(({ initTinysearch }) => initTinysearch());
    }
    return indexPromise;
  };

  const resultUrl = (value, query) => {
    const indexedUrl = new URL(value, location.origin);
    const localUrl = new URL(indexedUrl.pathname, location.origin);
    localUrl.searchParams.set("highlight", query);
    localUrl.hash = indexedUrl.hash;
    return `${localUrl.pathname}${localUrl.search}${localUrl.hash}`;
  };

  const renderSearchResults = async () => {
    const query = searchInput.value.trim();
    if (query.length < 2) {
      results.hidden = true;
      resultsList.replaceChildren();
      return;
    }

    try {
      const index = await loadIndex();
      if (query !== searchInput.value.trim()) return;
      const matches = index.search(query, Math.max(rows.length * 2, 8))
        .filter((match) => proposalPath(match.url));
      fullTextQuery = normalize(query);
      fullTextProposalPaths = new Set(matches.map((match) => proposalPath(match.url)));
      filterRows();

      const items = matches.slice(0, 8).map((match) => {
        const item = document.createElement("li");
        const link = document.createElement("a");
        const title = document.createElement("strong");
        const summary = document.createElement("small");
        const indexedUrl = new URL(match.url, location.origin);
        link.href = resultUrl(match.url, query);
        appendHighlightedText(title, match.title || indexedUrl.pathname, query);
        appendHighlightedText(
          summary,
          searchExcerpt(match.meta || indexedUrl.pathname, query),
          query,
        );
        link.append(title, summary);
        item.append(link);
        return item;
      });
      resultsList.replaceChildren(...items);
      results.hidden = items.length === 0;
    } catch (_) {
      fullTextQuery = "";
      fullTextProposalPaths.clear();
      results.hidden = true;
    }
  };

  searchInput.addEventListener("input", () => {
    fullTextQuery = "";
    fullTextProposalPaths.clear();
    filterRows();
    window.clearTimeout(debounceTimer);
    debounceTimer = window.setTimeout(renderSearchResults, 120);
  });
  statusFilter?.addEventListener("change", filterRows);
  facetFilter?.addEventListener("change", filterRows);

  document.addEventListener("keydown", (event) => {
    if (event.key === "/" && !event.ctrlKey && !event.metaKey && !event.altKey) {
      const target = event.target;
      if (!(target instanceof HTMLInputElement) && !(target instanceof HTMLTextAreaElement)) {
        event.preventDefault();
        searchInput.focus();
      }
    }
    if (event.key === "Escape") {
      results.hidden = true;
      searchInput.blur();
    }
  });

  document.addEventListener("click", (event) => {
    if (!results.contains(event.target) && event.target !== searchInput) results.hidden = true;
  });
})();
