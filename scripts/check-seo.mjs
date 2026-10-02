import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const siteNames = ["kips", "kccs"];
const siteOrigins = siteNames.map((name) => `https://${name}.dev`);
const checkLive = process.argv.includes("--live");
const checkLocal = process.argv.includes("--local");
const requestTimeoutMs = 20_000;

const namedEntities = {
  "&amp;": "&",
  "&quot;": '"',
  "&apos;": "'",
  "&lt;": "<",
  "&gt;": ">",
};

function decodeHtmlEntities(value) {
  const entityPattern = /&(?:amp|quot|apos|lt|gt|#\d+|#x[0-9a-f]+);/gi;

  return value.replace(entityPattern, (entity) => {
    if (namedEntities[entity] !== undefined) {
      return namedEntities[entity];
    }

    const isHexadecimal = entity.startsWith("&#x");
    const digits = entity.slice(isHexadecimal ? 3 : 2);
    const codePoint = parseInt(digits, isHexadecimal ? 16 : 10);
    return String.fromCodePoint(codePoint);
  });
}

// Zola's minified HTML contains both quoted and unquoted attribute values.
// These readers target the generated site output, not arbitrary HTML documents.
function parseAttributes(tag) {
  const attributePattern = /([\w:-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/g;
  const attributes = {};

  for (const match of tag.matchAll(attributePattern)) {
    const [, name, doubleQuoted, singleQuoted, unquoted] = match;
    attributes[name] = decodeHtmlEntities(doubleQuoted ?? singleQuoted ?? unquoted);
  }

  return attributes;
}

function readTagAttributes(html, tagName) {
  const tagPattern = new RegExp(`<${tagName}\\b[^>]*>`, "g");
  return [...html.matchAll(tagPattern)].map((match) => parseAttributes(match[0]));
}

function requestUrl(canonicalUrl) {
  if (!checkLocal) {
    return canonicalUrl;
  }

  return canonicalUrl
    .replace("https://kips.dev", "http://127.0.0.1:8788")
    .replace("https://kccs.dev", "http://127.0.0.1:8789");
}

const pages = new Map();
const redirects = new Map();

// Check each site's generated pages, then collect their URLs and fragment IDs.
for (const siteName of siteNames) {
  const origin = `https://${siteName}.dev`;
  const root = resolve("dist", siteName);
  const sitemap = await readFile(resolve(root, "sitemap.xml"), "utf8");
  const sitemapUrls = [...sitemap.matchAll(/<loc>(.*?)<\/loc>/g)]
    .map((match) => decodeHtmlEntities(match[1]));

  assert(sitemapUrls.length > 1, `${siteName}: empty sitemap`);
  assert.equal(new Set(sitemapUrls).size, sitemapUrls.length, `${siteName}: duplicate sitemap URL`);

  const robots = await readFile(resolve(root, "robots.txt"), "utf8");
  assert(robots.includes(`Sitemap: ${origin}/sitemap.xml`));

  const descriptions = new Set();
  const titles = new Set();

  for (const url of sitemapUrls) {
    const { origin: pageOrigin, pathname } = new URL(url);
    assert.equal(pageOrigin, origin, `Wrong sitemap domain: ${url}`);

    const pagePath = resolve(root, `.${pathname}`, "index.html");
    const html = await readFile(pagePath, "utf8");

    // Canonical URLs and metadata must agree with the sitemap.
    const canonicalLinks = readTagAttributes(html, "link")
      .filter((link) => link.rel === "canonical");
    assert.equal(canonicalLinks.length, 1, `${url}: canonical count`);
    assert.equal(canonicalLinks[0].href, url, `${url}: canonical mismatch`);

    const metaTags = readTagAttributes(html, "meta");
    const hasNoindex = metaTags.some(
      (meta) => meta.name === "robots" && /noindex/i.test(meta.content),
    );
    const openGraphUrl = metaTags.find((meta) => meta.property === "og:url")?.content;
    const description = metaTags.find((meta) => meta.name === "description")?.content;
    const title = decodeHtmlEntities(html.match(/<title>(.*?)<\/title>/s)?.[1] ?? "");
    const headingCount = [...html.matchAll(/<h1\b/g)].length;

    assert(!hasNoindex, `${url}: noindex`);
    assert.equal(openGraphUrl, url);
    assert(description?.length > 30, `${url}: missing useful description`);
    assert(!descriptions.has(description), `${url}: duplicate description`);
    assert(title.includes("Kaspa"), `${url}: title lacks Kaspa context`);
    assert(!titles.has(title), `${url}: duplicate title`);
    assert.equal(headingCount, 1, `${url}: expected one h1`);

    descriptions.add(description);
    titles.add(title);

    // JSON-LD can contain a single entity or several entities inside @graph.
    const schemas = [];
    const scriptPattern = /<script\b([^>]*)>([\s\S]*?)<\/script>/g;
    for (const [, attributes, content] of html.matchAll(scriptPattern)) {
      if (parseAttributes(attributes).type === "application/ld+json") {
        schemas.push(JSON.parse(content));
      }
    }

    assert(schemas.length > 0, `${url}: missing structured data`);
    const entities = schemas.flatMap((schema) => schema["@graph"] ?? [schema]);

    if (pathname === "/") {
      const hasWebsite = entities.some(
        (entity) => entity["@type"] === "WebSite" && entity.url === url,
      );
      assert(hasWebsite);
    } else {
      const article = entities.find((entity) => entity["@type"] === "TechArticle");
      assert.equal(article?.url, url);
      assert.equal(article.description, description);
      assert(article.author.every(
        (author) => typeof author.name === "string" && !author.email,
      ));

      const breadcrumb = entities.find((entity) => entity["@type"] === "BreadcrumbList");
      assert.equal(breadcrumb?.itemListElement.at(-1)?.item, url);
      for (const item of breadcrumb.itemListElement) {
        assert(sitemapUrls.includes(item.item), `${url}: invalid breadcrumb ${item.item}`);
      }
    }

    const fragmentIds = new Set();
    const idPattern = /\bid=(?:"([^"]+)"|'([^']+)'|([^\s>]+))/g;
    for (const [, doubleQuoted, singleQuoted, unquoted] of html.matchAll(idPattern)) {
      fragmentIds.add(decodeHtmlEntities(doubleQuoted ?? singleQuoted ?? unquoted));
    }
    pages.set(url, { html, fragmentIds });
  }

  const redirectFile = await readFile(resolve(root, "_redirects"), "utf8");
  for (const line of redirectFile.trim().split(/\r?\n/)) {
    if (!line || line.startsWith("#")) {
      continue;
    }

    const [from, to, statusCode] = line.split(/\s+/);
    const sourceUrl = origin + from;
    const destinationUrl = origin + to;

    assert.equal(statusCode, "301");
    assert(!redirects.has(sourceUrl), `Duplicate redirect ${from}`);
    assert(sitemapUrls.includes(destinationUrl), `Redirect target not canonical: ${to}`);
    redirects.set(sourceUrl, destinationUrl);
  }

  // CI removes this intermediate JSON file after compiling the search index.
  const searchIndexPath = resolve(root, "search_index.en.json");
  try {
    const searchDocuments = JSON.parse(await readFile(searchIndexPath, "utf8"));
    for (const document of searchDocuments) {
      assert(sitemapUrls.includes(document.url), `Noncanonical search URL: ${document.url}`);
    }
  } catch (error) {
    if (error.code !== "ENOENT") {
      throw error;
    }
  }

  console.log(`${siteName}: ${sitemapUrls.length} canonical pages, unique metadata, valid structured data`);
}

// Both sites must be loaded before checking links between them.
const brokenFragments = [];
const missingTargets = [];

for (const [url, { html }] of pages) {
  for (const match of html.matchAll(/<(?:a|img)\b[^>]*>/g)) {
    const attributes = parseAttributes(match[0]);
    const href = attributes.href ?? attributes.src;
    if (!href) {
      continue;
    }

    const target = new URL(href, url);
    if (!siteOrigins.includes(target.origin)) {
      continue;
    }

    const targetUrl = target.origin + target.pathname;
    assert(!redirects.has(targetUrl), `${url} links through redirect ${targetUrl}`);

    const targetPage = pages.get(targetUrl);
    if (targetPage) {
      const fragment = decodeURIComponent(target.hash.slice(1));
      if (target.hash && !targetPage.fragmentIds.has(fragment)) {
        brokenFragments.push(`${url} -> ${target.href}`);
      }
      continue;
    }

    const targetSite = target.hostname.split(".")[0];
    const assetPath = resolve("dist", targetSite, `.${decodeURIComponent(target.pathname)}`);
    const filePath = target.pathname.endsWith("/")
      ? resolve(assetPath, "index.html")
      : assetPath;

    try {
      await readFile(filePath);
    } catch (error) {
      if (error.code !== "ENOENT") {
        throw error;
      }
      missingTargets.push(`${url} -> ${targetUrl}`);
    }
  }
}

assert.deepEqual(brokenFragments, [], "Broken internal fragments");
assert.deepEqual(missingTargets, [], "Missing internal pages or assets");

// Optionally verify HTTP behavior against production or local Wrangler servers.
if (checkLive || checkLocal) {
  for (const url of pages.keys()) {
    const response = await fetch(requestUrl(url), {
      redirect: "manual",
      signal: AbortSignal.timeout(requestTimeoutMs),
    });
    assert.equal(response.status, 200, `${url}: live status`);

    const robotsHeader = response.headers.get("x-robots-tag") ?? "";
    assert(!/noindex/i.test(robotsHeader));

    const html = await response.text();
    const canonical = readTagAttributes(html, "link")
      .find((link) => link.rel === "canonical");
    assert.equal(canonical?.href, url, `${url}: live canonical`);
  }

  for (const [from, to] of redirects) {
    const response = await fetch(requestUrl(`${from}?highlight=covenant`), {
      redirect: "manual",
      signal: AbortSignal.timeout(requestTimeoutMs),
    });
    assert.equal(response.status, 301, `${from}: live redirect status`);

    const location = response.headers.get("location");
    const actualDestination = new URL(location, requestUrl(from)).href;
    const expectedDestination = requestUrl(`${to}?highlight=covenant`);
    assert.equal(
      actualDestination,
      expectedDestination,
      `${from}: redirect lost query or wrong destination`,
    );
  }

  for (const siteName of siteNames) {
    const missingPageUrl = requestUrl(`https://${siteName}.dev/seo-check-nonexistent/`);
    const response = await fetch(missingPageUrl, { redirect: "manual" });
    assert.equal(response.status, 404);
  }

  const environment = checkLocal ? "Local Cloudflare emulation" : "Live";
  console.log(`${environment}: pages, permanent redirects, query preservation, and 404s verified.`);
}

console.log(`SEO checks passed: ${pages.size} pages and ${redirects.size} permanent redirects.`);
