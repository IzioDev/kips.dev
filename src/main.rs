use anyhow::{Context, Result, bail};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

const BARE_URL_PATTERN: &str =
    r"(?m)(?P<prefix>^|[ \t])(?P<url>https?://[^\s<>()]*[A-Za-z0-9/#=_~+%-])";
const BARE_URL_REPLACEMENT: &str = "${prefix}<${url}>";

// A persistent registry owns published URLs independently of mutable source titles.
#[derive(Debug, Deserialize)]
#[serde(transparent)]
struct SeoCatalog(BTreeMap<String, SeoDocument>);

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SeoDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    slug: Option<String>,
    description: String,
}

impl SeoCatalog {
    // Only new documents are recorded; published URLs and curated text never drift.
    fn discover(&mut self, proposals: &[Proposal]) -> Result<String> {
        let mut additions = BTreeMap::new();
        for proposal in proposals {
            let key = format!("{}-{:04}", proposal.kind.file_prefix(), proposal.number);
            if !self.0.contains_key(&key) {
                let title_slug = proposal
                    .title
                    .split(|character: char| !character.is_ascii_alphanumeric())
                    .filter(|word| !word.is_empty())
                    .collect::<Vec<_>>()
                    .join("-")
                    .to_ascii_lowercase();
                additions.insert(
                    key.clone(),
                    SeoDocument {
                        slug: Some(format!(
                            "{}-{}-{}",
                            proposal.kind.file_prefix(),
                            proposal.number,
                            if title_slug.is_empty() {
                                "proposal"
                            } else {
                                &title_slug
                            }
                        )),
                        description: format!(
                            "Kaspa {} {}: {}. Read the specification and proposal status.",
                            proposal.kind.prefix(),
                            proposal.number,
                            proposal.title
                        ),
                    },
                );
            }
            for companion in &proposal.companions {
                let key = format!("{key}/{}", companion.route);
                if !self.0.contains_key(&key) {
                    additions.insert(
                        key,
                        SeoDocument {
                            slug: None,
                            description: format!(
                                "Kaspa {} {} companion: {}.",
                                proposal.kind.prefix(),
                                proposal.number,
                                companion.title
                            ),
                        },
                    );
                }
            }
        }
        if additions.is_empty() {
            return Ok(String::new());
        }
        let text = toml::to_string(&additions)?;
        Self::parse(&text)?;
        self.0.extend(additions);
        Ok(format!("\n{text}"))
    }

    fn parse(source: &str) -> Result<Self> {
        let catalog: Self = toml::from_str(source).context("invalid site/seo.toml")?;
        let key_pattern = Regex::new(r"^(kip|kcc)-[0-9]{4,}(?:/[a-z0-9_-]+(?:/[a-z0-9_-]+)*)?$")?;
        let slug_pattern = Regex::new(r"^(kip|kcc)-[0-9]+-[a-z0-9]+(?:-[a-z0-9]+)*$")?;
        let mut routes = BTreeSet::new();
        for (key, document) in &catalog.0 {
            if !key_pattern.is_match(key) || document.description.trim().is_empty() {
                bail!("invalid SEO document key or empty description: {key}");
            }
            if let Some(slug) = &document.slug {
                let (prefix, number) = key.split_once('-').context("missing proposal prefix")?;
                let number = number
                    .parse::<u32>()
                    .context("only main documents may define slugs")?;
                if !slug_pattern.is_match(slug)
                    || !slug.starts_with(&format!("{prefix}-{number}-"))
                    || !routes.insert(slug)
                {
                    bail!("invalid or duplicate published slug for {key}: {slug}");
                }
            }
        }
        Ok(catalog)
    }

    fn path(&self, kind: ProposalKind, number: u32) -> String {
        let key = format!("{}-{number:04}", kind.file_prefix());
        match self
            .0
            .get(&key)
            .and_then(|document| document.slug.as_deref())
        {
            Some(slug) => format!("/{slug}/"),
            None => format!("/{number}/"),
        }
    }
}

fn main() -> Result<()> {
    SiteGenerator::from_env()?.run()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProposalKind {
    Kip,
    Kcc,
}

impl ProposalKind {
    fn from_prefix(prefix: &str) -> Option<Self> {
        match prefix.to_ascii_lowercase().as_str() {
            "kip" => Some(Self::Kip),
            "kcc" => Some(Self::Kcc),
            _ => None,
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Kip => "KIP",
            Self::Kcc => "KCC",
        }
    }

    fn file_prefix(self) -> &'static str {
        match self {
            Self::Kip => "kip",
            Self::Kcc => "kcc",
        }
    }

    fn plural(self) -> &'static str {
        match self {
            Self::Kip => "KIPs",
            Self::Kcc => "KCCs",
        }
    }

    fn long_name(self) -> &'static str {
        match self {
            Self::Kip => "Kaspa Improvement Proposals",
            Self::Kcc => "Kaspa Calls for Conventions",
        }
    }

    fn domain(self) -> &'static str {
        match self {
            Self::Kip => "kips.dev",
            Self::Kcc => "kccs.dev",
        }
    }

    fn repository(self) -> &'static str {
        match self {
            Self::Kip => "kips",
            Self::Kcc => "kccs",
        }
    }

    fn branch(self) -> &'static str {
        match self {
            Self::Kip => "master",
            Self::Kcc => "main",
        }
    }

    fn summary(self) -> &'static str {
        match self {
            Self::Kip => {
                "Browse Kaspa Improvement Proposals (KIPs) for consensus, network upgrades, node behavior, and client APIs, with specifications and proposal statuses."
            }
            Self::Kcc => {
                "Browse Kaspa Calls for Conventions (KCCs) for covenants, token standards, application interfaces, wallets, and interoperable ecosystem tooling."
            }
        }
    }

    fn facet_label(self) -> &'static str {
        match self {
            Self::Kip => "Layer",
            Self::Kcc => "Type",
        }
    }

    fn counterpart(self) -> Self {
        match self {
            Self::Kip => Self::Kcc,
            Self::Kcc => Self::Kip,
        }
    }

    fn source_url(self, relative_path: &str) -> String {
        format!(
            "https://github.com/kaspanet/{}/blob/{}/{}",
            self.repository(),
            self.branch(),
            relative_path.replace('\\', "/")
        )
    }
}

#[derive(Debug)]
enum ProposalSource {
    Upstream(PathBuf),
    Directory(PathBuf),
}

impl ProposalSource {
    fn last_modified(&self, path: &Path) -> Result<Option<toml::value::Datetime>> {
        if !self.path().join(".git").exists() {
            return Ok(None);
        }
        let relative = path.strip_prefix(self.path())?;
        let output = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(["log", "-1", "--format=%cs", "--"])
            .arg(relative)
            .output()
            .context("could not read document modification date")?;
        if !output.status.success() {
            bail!("could not read modification date for {}", path.display());
        }
        Ok(MetadataBlock::date(
            String::from_utf8(output.stdout)?.trim(),
        ))
    }

    fn path(&self) -> &Path {
        match self {
            Self::Upstream(path) | Self::Directory(path) => path,
        }
    }

    fn prepare(&self, kind: ProposalKind) -> Result<()> {
        let path = self.path();
        if matches!(self, Self::Directory(_)) {
            if !path.is_dir() {
                bail!(
                    "{} source directory does not exist: {}",
                    kind.prefix(),
                    path.display()
                );
            }
            return Ok(());
        }

        let repository_url = format!("https://github.com/kaspanet/{}.git", kind.repository());
        if !path.exists() {
            let parent = path
                .parent()
                .with_context(|| format!("could not resolve the parent of {}", path.display()))?;
            fs::create_dir_all(parent)
                .with_context(|| format!("could not create {}", parent.display()))?;
            let status = Command::new("git")
                .args(["clone", "--branch", kind.branch(), "--single-branch"])
                .arg(&repository_url)
                .arg(path)
                .status()
                .with_context(|| format!("could not start git to clone {}", kind.plural()))?;
            if !status.success() {
                bail!("could not clone {repository_url} into {}", path.display());
            }
            return Ok(());
        }

        if !path.join(".git").is_dir() {
            bail!(
                "managed {} source is not a Git checkout: {}",
                kind.prefix(),
                path.display()
            );
        }

        let remote = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["remote", "get-url", "origin"])
            .output()
            .with_context(|| format!("could not inspect {} upstream", kind.plural()))?;
        if !remote.status.success() {
            bail!("could not inspect the Git origin in {}", path.display());
        }
        let remote_url =
            String::from_utf8(remote.stdout).context("the Git origin URL was not valid UTF-8")?;
        if remote_url.trim() != repository_url {
            bail!(
                "managed {} source has unexpected origin {}; expected {}",
                kind.prefix(),
                remote_url.trim(),
                repository_url
            );
        }

        let run_git = |arguments: &[&str], action: &str| -> Result<()> {
            let status = Command::new("git")
                .arg("-C")
                .arg(path)
                .args(arguments)
                .status()
                .with_context(|| format!("could not start git to {action}"))?;
            if !status.success() {
                bail!("could not {action} in {}", path.display());
            }
            Ok(())
        };
        run_git(
            &["fetch", "origin", kind.branch()],
            &format!("fetch current {}", kind.plural()),
        )?;
        run_git(
            &["merge", "--ff-only", "FETCH_HEAD"],
            &format!("fast-forward current {}", kind.plural()),
        )
    }
}

#[derive(Debug)]
struct GeneratorConfig {
    kips: ProposalSource,
    kccs: ProposalSource,
    output: PathBuf,
    shared_site: PathBuf,
}

impl GeneratorConfig {
    fn from_env() -> Result<Self> {
        let mut kips = ProposalSource::Upstream(PathBuf::from(".sources/kips"));
        let mut kccs = ProposalSource::Upstream(PathBuf::from(".sources/kccs"));
        let mut output = PathBuf::from(".generated");
        let mut args = env::args().skip(1);

        while let Some(flag) = args.next() {
            let value = args
                .next()
                .with_context(|| format!("missing value after {flag}"))?;
            match flag.as_str() {
                "--kips" => kips = ProposalSource::Directory(PathBuf::from(value)),
                "--kccs" => kccs = ProposalSource::Directory(PathBuf::from(value)),
                "--output" => output = PathBuf::from(value),
                _ => bail!("unknown option {flag}"),
            }
        }

        let shared_site = PathBuf::from("site");
        if !shared_site.is_dir() {
            bail!(
                "shared site directory does not exist: {}",
                shared_site.display()
            );
        }

        Ok(Self {
            kips,
            kccs,
            output,
            shared_site,
        })
    }

    fn source_for(&self, kind: ProposalKind) -> &ProposalSource {
        match kind {
            ProposalKind::Kip => &self.kips,
            ProposalKind::Kcc => &self.kccs,
        }
    }
}

struct SiteGenerator {
    config: GeneratorConfig,
    seo: SeoCatalog,
    markdown_link: Regex,
    bare_url: Regex,
    proposal_target: Regex,
    internal_anchor_link: Regex,
    reference_anchor: Regex,
    plain_proposal_reference: Regex,
}

impl SiteGenerator {
    fn from_env() -> Result<Self> {
        let config = GeneratorConfig::from_env()?;
        let seo_path = config.shared_site.join("seo.toml");
        let seo_source = match fs::read_to_string(&seo_path) {
            Ok(source) => source,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error).context("could not read site/seo.toml"),
        };
        Ok(Self {
            config,
            seo: SeoCatalog::parse(&seo_source)?,
            markdown_link: Regex::new(
                r"(?P<open>!?\[[^\]]*\]\()(?P<target><[^>\n]+>|[^)\s\n]+)(?P<close>\))",
            )?,
            bare_url: Regex::new(BARE_URL_PATTERN)?,
            proposal_target: Regex::new(r"(?i)(?:^|/)(kip|kcc)-0*([0-9]+)\.md$")?,
            internal_anchor_link: Regex::new(r"\]\(#([A-Za-z0-9_-]+)\)")?,
            reference_anchor: Regex::new(r#"<a id="ref-[0-9]+"></a>\[([0-9]+)\]"#)?,
            plain_proposal_reference: Regex::new(
                r"(?mi)^(?P<prefix>\s*\[[0-9]+\]\s+)(?P<kind>KIP|KCC)-0*(?P<number>[0-9]+)(?P<suffix>:)",
            )?,
        })
    }

    fn run(&mut self) -> Result<()> {
        for kind in [ProposalKind::Kip, ProposalKind::Kcc] {
            self.config.source_for(kind).prepare(kind)?;
        }
        self.clean_output()?;

        let collections = [
            (ProposalKind::Kip, self.load_proposals(ProposalKind::Kip)?),
            (ProposalKind::Kcc, self.load_proposals(ProposalKind::Kcc)?),
        ];
        // Discover both collections before rewriting any cross-site links.
        let mut additions = String::new();
        for (_, proposals) in &collections {
            additions.push_str(&self.seo.discover(proposals)?);
        }
        if !additions.is_empty() {
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.config.shared_site.join("seo.toml"))?
                .write_all(additions.as_bytes())
                .context("could not append discovered documents to site/seo.toml")?;
        }

        for (kind, proposals) in &collections {
            self.write_site(*kind, proposals)?;
            println!(
                "generated {} {} into {}",
                proposals.len(),
                kind.plural(),
                self.config.output.join(kind.repository()).display()
            );
        }

        Ok(())
    }

    fn clean_output(&self) -> Result<()> {
        let current_dir = env::current_dir().context("could not resolve the working directory")?;
        let output = if self.config.output.is_absolute() {
            self.config.output.clone()
        } else {
            current_dir.join(&self.config.output)
        };

        if output.file_name().and_then(|name| name.to_str()) != Some(".generated") {
            bail!(
                "refusing to clean an output directory not named .generated: {}",
                output.display()
            );
        }

        if output.exists() {
            fs::remove_dir_all(&output)
                .with_context(|| format!("could not clean {}", output.display()))?;
        }
        fs::create_dir_all(&output)
            .with_context(|| format!("could not create {}", output.display()))?;
        Ok(())
    }

    fn load_proposals(&self, kind: ProposalKind) -> Result<Vec<Proposal>> {
        let source_root = self.config.source_for(kind).path();
        let proposal_file = Regex::new(&format!(
            r"^{}-([0-9]+)\.md$",
            regex::escape(kind.file_prefix())
        ))?;
        let mut proposals = Vec::new();
        let mut numbers = BTreeSet::new();

        for entry in fs::read_dir(source_root)
            .with_context(|| format!("could not read {}", source_root.display()))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            let Some(captures) = proposal_file.captures(&file_name) else {
                continue;
            };
            let number = captures[1]
                .parse::<u32>()
                .with_context(|| format!("invalid proposal number in {file_name}"))?;
            if !numbers.insert(number) {
                bail!("duplicate {} number {number}", kind.prefix());
            }

            let mut proposal = Proposal::load(kind, number, entry.path())?;
            proposal.modified = self
                .config
                .source_for(kind)
                .last_modified(&proposal.source_path)?
                .or_else(|| MetadataBlock::date(&proposal.updated));
            for companion in &mut proposal.companions {
                let companion_path = source_root
                    .join(&proposal.source_stem)
                    .join(&companion.relative);
                companion.modified = self
                    .config
                    .source_for(kind)
                    .last_modified(&companion_path)?;
            }
            proposals.push(proposal);
        }

        proposals.sort_by_key(|proposal| proposal.number);
        if proposals.is_empty() {
            bail!(
                "no {} documents found in {}",
                kind.plural(),
                source_root.display()
            );
        }
        Ok(proposals)
    }

    fn write_site(&self, kind: ProposalKind, proposals: &[Proposal]) -> Result<()> {
        let site_root = self.config.output.join(kind.repository());
        let content_root = site_root.join("content");
        fs::create_dir_all(&content_root)?;
        self.copy_tree(
            &self.config.shared_site.join("templates"),
            &site_root.join("templates"),
        )?;
        self.copy_tree(
            &self.config.shared_site.join("static"),
            &site_root.join("static"),
        )?;

        fs::write(site_root.join("zola.toml"), self.site_config(kind))?;
        fs::write(
            content_root.join("_index.md"),
            self.section_front_matter(kind, proposals)?,
        )?;

        let mut redirects = String::new();
        for (index, proposal) in proposals.iter().enumerate() {
            self.copy_auxiliary_assets(proposal, &site_root.join("static"))?;

            let previous = index.checked_sub(1).map(|position| &proposals[position]);
            let next = proposals.get(index + 1);
            let body =
                self.normalize_heading_ids(&self.rewrite_links(proposal, &proposal.body, None));
            let front_matter = proposal.front_matter(previous, next, None, &self.seo);
            for alias in &front_matter.aliases {
                let destination = format!("/{}/", front_matter.path);
                redirects.push_str(&format!(
                    "{alias} {destination} 301\n{} {destination} 301\n",
                    alias.trim_end_matches('/')
                ));
            }
            fs::write(
                content_root.join(format!("{:04}-main.md", proposal.number)),
                format!("+++\n{}+++\n\n{}\n", toml::to_string(&front_matter)?, body),
            )?;

            for companion in &proposal.companions {
                let body = self.normalize_heading_ids(&self.rewrite_links(
                    proposal,
                    &companion.body,
                    Some(companion),
                ));
                let front_matter = proposal.front_matter(None, None, Some(companion), &self.seo);
                for alias in &front_matter.aliases {
                    let destination = format!("/{}/", front_matter.path);
                    redirects.push_str(&format!(
                        "{alias} {destination} 301\n{} {destination} 301\n",
                        alias.trim_end_matches('/')
                    ));
                }
                let safe_name = companion.route.replace('/', "-");
                fs::write(
                    content_root.join(format!("{:04}-companion-{}.md", proposal.number, safe_name)),
                    format!("+++\n{}+++\n\n{}\n", toml::to_string(&front_matter)?, body),
                )?;
            }
        }

        fs::write(site_root.join("static/_redirects"), redirects)?;

        Ok(())
    }

    fn site_config(&self, kind: ProposalKind) -> String {
        let counterpart = kind.counterpart();
        format!(
            r#"base_url = "https://{}"
title = "{} ({}) | {}.dev"
description = "{}"
default_language = "en"
compile_sass = false
minify_html = true
build_search_index = true
generate_sitemap = true
generate_robots_txt = true
skip_content_templating = ["**/*.md"]

[markdown]
external_links_target_blank = true
external_links_no_referrer = true
github_alerts = true
bottom_footnotes = true
insert_anchor_links = "heading"
lazy_async_image = true

[markdown.highlighting]
theme = "github-dark"
error_on_missing_language = false

[link_checker]
internal_level = "warn"
external_level = "warn"

[search]
include_title = true
include_description = true
include_path = true
include_content = true
index_format = "fuse_json"

[extra]
proposal_kind = "{}"
proposal_plural = "{}"
long_name = "{}"
facet_label = "{}"
source_url = "https://github.com/kaspanet/{}"
counterpart_url = "https://{}"
counterpart_label = "{}"
"#,
            kind.domain(),
            kind.long_name(),
            kind.plural(),
            kind.plural(),
            kind.summary(),
            kind.prefix(),
            kind.plural(),
            kind.long_name(),
            kind.facet_label(),
            kind.repository(),
            counterpart.domain(),
            counterpart.plural(),
        )
    }

    fn section_front_matter(&self, kind: ProposalKind, proposals: &[Proposal]) -> Result<String> {
        let statuses = proposals
            .iter()
            .map(|proposal| proposal.status.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let facets = proposals
            .iter()
            .flat_map(|proposal| proposal.facets.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let front_matter = SectionFrontMatter {
            title: kind.long_name().to_string(),
            description: kind.summary().to_string(),
            template: "index.html",
            sort_by: "weight",
            extra: SectionExtra {
                proposal_count: proposals.len(),
                statuses,
                facets,
            },
        };
        Ok(format!(
            "+++\n{}+++\n\n{}\n",
            toml::to_string(&front_matter)?,
            kind.summary()
        ))
    }

    fn rewrite_links(
        &self,
        proposal: &Proposal,
        body: &str,
        companion: Option<&Companion>,
    ) -> String {
        let body = self
            .markdown_link
            .replace_all(body, |captures: &Captures<'_>| {
                let original_target = &captures["target"];
                let unwrapped = original_target
                    .strip_prefix('<')
                    .and_then(|value| value.strip_suffix('>'))
                    .unwrap_or(original_target);
                let (target_path, fragment) = unwrapped
                    .split_once('#')
                    .map_or((unwrapped, ""), |(path, anchor)| (path, anchor));

                let rewritten = if let Some(reference) = self.proposal_target.captures(target_path)
                {
                    let Some(target_kind) = ProposalKind::from_prefix(&reference[1]) else {
                        return captures[0].to_string();
                    };
                    let number = reference[2].parse::<u32>().unwrap_or_default();
                    let base = if target_kind == proposal.kind {
                        String::new()
                    } else {
                        format!("https://{}", target_kind.domain())
                    };
                    format!(
                        "{base}{}{}",
                        self.seo.path(target_kind, number),
                        if fragment.is_empty() {
                            String::new()
                        } else {
                            format!("#{fragment}")
                        }
                    )
                } else if target_path.contains("://")
                    || target_path.starts_with("mailto:")
                    || target_path.starts_with('/')
                    || target_path.is_empty()
                {
                    return captures[0].to_string();
                } else if let Some(relative) =
                    proposal.resolve_auxiliary_target(target_path, companion)
                {
                    let anchor = if fragment.is_empty() {
                        String::new()
                    } else {
                        format!("#{fragment}")
                    };
                    if relative
                        .extension()
                        .and_then(|extension| extension.to_str())
                        == Some("md")
                    {
                        let route = relative.with_extension("");
                        format!(
                            "{}{}/{}",
                            self.seo.path(proposal.kind, proposal.number),
                            Proposal::web_path(&route),
                            anchor
                        )
                    } else {
                        format!(
                            "/{}/{}{}",
                            proposal.number,
                            Proposal::web_path(&relative),
                            anchor
                        )
                    }
                } else if !target_path.contains('/') && target_path.ends_with(".md") {
                    let source_url = proposal.kind.source_url(target_path);
                    if fragment.is_empty() {
                        source_url
                    } else {
                        format!("{source_url}#{fragment}")
                    }
                } else {
                    return captures[0].to_string();
                };

                format!("{}{}{}", &captures["open"], rewritten, &captures["close"])
            })
            .into_owned();

        let body = self
            .plain_proposal_reference
            .replace_all(&body, |captures: &Captures<'_>| {
                let Some(target_kind) = ProposalKind::from_prefix(&captures["kind"]) else {
                    return captures[0].to_string();
                };
                let Ok(number) = captures["number"].parse::<u32>() else {
                    return captures[0].to_string();
                };
                let base = if target_kind == proposal.kind {
                    String::new()
                } else {
                    format!("https://{}", target_kind.domain())
                };

                format!(
                    "{}[{}-{number}]({base}{}){}",
                    &captures["prefix"],
                    target_kind.prefix(),
                    self.seo.path(target_kind, number),
                    &captures["suffix"]
                )
            })
            .into_owned();

        self.bare_url
            .replace_all(&body, BARE_URL_REPLACEMENT)
            .into_owned()
    }

    fn normalize_heading_ids(&self, body: &str) -> String {
        let body = self
            .reference_anchor
            .replace_all(body, |captures: &Captures<'_>| {
                format!("<a id=\"ref-{}\"></a>[{}]", &captures[1], &captures[1])
            })
            .into_owned();
        let requested_anchors = self
            .internal_anchor_link
            .captures_iter(&body)
            .map(|captures| captures[1].to_string())
            .collect::<BTreeSet<_>>();
        let mut fence: Option<(char, usize)> = None;
        let mut seen = BTreeMap::<String, usize>::new();
        let mut output = Vec::new();
        let mut headings = Vec::new();

        for line in body.lines() {
            let trimmed = line.trim_start();
            let fence_character = trimmed.chars().next().unwrap_or(' ');
            let fence_length = trimmed
                .chars()
                .take_while(|character| *character == fence_character)
                .count();
            if matches!(fence_character, '`' | '~') && fence_length >= 3 {
                match fence {
                    None => fence = Some((fence_character, fence_length)),
                    Some((character, length))
                        if character == fence_character
                            && fence_length >= length
                            && trimmed[fence_length..].trim().is_empty() =>
                    {
                        fence = None
                    }
                    _ => {}
                }
                output.push(line.to_string());
                continue;
            }
            if fence.is_some() || line.starts_with("    ") || line.starts_with('\t') {
                output.push(line.to_string());
                continue;
            }

            let marker_length = line
                .chars()
                .take_while(|character| *character == '#')
                .count();
            if !(1..=6).contains(&marker_length) || line.chars().nth(marker_length) != Some(' ') {
                output.push(line.to_string());
                continue;
            }

            // The document template owns h1. Preserve source anchor IDs while nesting
            // body headings beneath it, including explicitly assigned anchors.
            if line.contains("{#") {
                headings.push((output.len(), marker_length));
                output.push(line.to_string());
                continue;
            }

            let title = line[marker_length + 1..]
                .trim()
                .trim_end_matches('#')
                .trim();
            let mut slug = String::new();
            let mut previous_was_space = false;
            for character in title.chars().flat_map(char::to_lowercase) {
                if character.is_alphanumeric() || character == '-' || character == '_' {
                    slug.push(character);
                    previous_was_space = false;
                } else if character.is_whitespace() && !previous_was_space && !slug.is_empty() {
                    slug.push('-');
                    previous_was_space = true;
                }
            }
            while slug.ends_with('-') {
                slug.pop();
            }
            if slug.is_empty() {
                headings.push((output.len(), marker_length));
                output.push(line.to_string());
                continue;
            }

            let duplicate = seen.entry(slug.clone()).or_default();
            let unique_slug = if *duplicate == 0 {
                slug
            } else {
                format!("{slug}-{duplicate}")
            };
            *duplicate += 1;

            let compact_slug = unique_slug.replace(['-', '_'], "");
            for alias in requested_anchors.iter().filter(|anchor| {
                *anchor != &unique_slug && anchor.replace(['-', '_'], "") == compact_slug
            }) {
                output.push(format!("<span id=\"{alias}\"></span>"));
            }
            headings.push((output.len(), marker_length));
            output.push(format!("{line} {{#{unique_slug}}}"));
        }

        if headings.iter().any(|(_, level)| *level == 1) {
            for (index, level) in headings {
                if level < 6 {
                    output[index].insert(0, '#');
                }
            }
        }

        output.join("\n")
    }

    fn copy_auxiliary_assets(&self, proposal: &Proposal, static_root: &Path) -> Result<()> {
        let Some(auxiliary_root) = &proposal.auxiliary_root else {
            return Ok(());
        };
        let destination_root = static_root.join(proposal.number.to_string());

        for entry in WalkDir::new(auxiliary_root).follow_links(false) {
            let entry = entry?;
            if !entry.file_type().is_file()
                || entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("md")
            {
                continue;
            }
            let relative = entry.path().strip_prefix(auxiliary_root)?;
            let destination = destination_root.join(relative);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &destination).with_context(|| {
                format!(
                    "could not copy auxiliary asset {} to {}",
                    entry.path().display(),
                    destination.display()
                )
            })?;
        }
        Ok(())
    }

    fn copy_tree(&self, source: &Path, destination: &Path) -> Result<()> {
        for entry in WalkDir::new(source).follow_links(false) {
            let entry = entry?;
            let relative = entry.path().strip_prefix(source)?;
            let output = destination.join(relative);
            if entry.file_type().is_dir() {
                fs::create_dir_all(&output)?;
            } else if entry.file_type().is_file() {
                if let Some(parent) = output.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(entry.path(), &output).with_context(|| {
                    format!(
                        "could not copy {} to {}",
                        entry.path().display(),
                        output.display()
                    )
                })?;
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct MetadataBlock {
    fields: BTreeMap<String, String>,
    body: String,
}

impl MetadataBlock {
    fn date(value: &str) -> Option<toml::value::Datetime> {
        let parsed = value.parse::<toml::value::Datetime>().ok()?;
        let date = parsed.date?;
        if parsed.time.is_some() || parsed.offset.is_some() {
            return None;
        }
        let leap = date.year.is_multiple_of(4)
            && (!date.year.is_multiple_of(100) || date.year.is_multiple_of(400));
        let days = match date.month {
            2 if leap => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            _ => return None,
        };
        (date.day > 0 && date.day <= days).then_some(parsed)
    }

    fn summary(body: &str, fallback: &str) -> String {
        let mut fence: Option<(char, usize)> = None;
        let mut paragraph = Vec::new();
        for line in body.lines().chain(std::iter::once("")) {
            let trimmed = line.trim();
            let character = trimmed.chars().next().unwrap_or(' ');
            let length = trimmed
                .chars()
                .take_while(|value| *value == character)
                .count();
            if matches!(character, '`' | '~') && length >= 3 {
                match fence {
                    None => fence = Some((character, length)),
                    Some((open, count))
                        if open == character
                            && length >= count
                            && trimmed[length..].trim().is_empty() =>
                    {
                        fence = None
                    }
                    _ => {}
                }
                continue;
            }
            if fence.is_some() {
                continue;
            }
            if trimmed.is_empty() {
                if !paragraph.is_empty() {
                    break;
                }
                continue;
            }
            if trimmed.starts_with(['#', '-', '*', '>', '|', '<', '['])
                || trimmed.starts_with("Status:")
                || line.starts_with("    ")
                || line.starts_with('\t')
                || trimmed
                    .chars()
                    .next()
                    .is_some_and(|value| value.is_ascii_digit())
            {
                continue;
            }
            paragraph.push(trimmed);
        }
        let prose = paragraph.join(" ");
        let mut result = String::new();
        for word in prose.split_whitespace() {
            // If markup needs interpretation, use the safe title/context fallback.
            if word.contains(['[', ']', '<', '>']) {
                return fallback.to_string();
            }
            let word = word.replace(['*', '`'], "");
            if result.chars().count() + word.chars().count() + 1 > 180 {
                result.push('…');
                break;
            }
            if !result.is_empty() {
                result.push(' ');
            }
            result.push_str(&word);
        }
        if result.is_empty() {
            fallback.to_string()
        } else {
            result
        }
    }

    fn parse(source: &str, path: &Path) -> Result<Self> {
        let source = source.trim_start_matches('\u{feff}');
        let mut lines = source.lines();
        if lines.next().map(str::trim) != Some("```") {
            bail!(
                "{} does not start with a fenced metadata block",
                path.display()
            );
        }

        let mut fields = BTreeMap::new();
        let mut found_end = false;
        for line in lines.by_ref() {
            if line.trim() == "```" {
                found_end = true;
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                fields.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
        if !found_end {
            bail!("{} has an unterminated metadata block", path.display());
        }

        Ok(Self {
            fields,
            body: lines.collect::<Vec<_>>().join("\n").trim().to_string(),
        })
    }

    fn required(&self, keys: &[&str], path: &Path) -> Result<String> {
        self.optional(keys).with_context(|| {
            format!(
                "{} is missing required metadata ({})",
                path.display(),
                keys.join(" or ")
            )
        })
    }

    fn optional(&self, keys: &[&str]) -> Option<String> {
        keys.iter()
            .find_map(|key| self.fields.get(*key))
            .filter(|value| !value.trim().is_empty())
            .cloned()
    }
}

#[derive(Clone, Debug, Serialize)]
struct Author {
    raw: String,
    name: String,
    email: String,
    username: String,
}

impl Author {
    fn parse(value: &str) -> Self {
        let raw = value.trim().to_string();
        let mut name = raw.clone();
        let mut email = String::new();
        let mut username = String::new();

        if let Some(start) = name.rfind('<')
            && let Some(relative_end) = name[start + 1..].find('>')
        {
            let end = start + 1 + relative_end;
            email = name[start + 1..end].trim().to_string();
            name.replace_range(start..=end, " ");
        }

        if let Some(start) = name.find("(@")
            && let Some(relative_end) = name[start + 2..].find(')')
        {
            let end = start + 2 + relative_end;
            username = name[start + 1..end].trim().to_string();
            name.replace_range(start..=end, " ");
        } else if let Some(handle) = name
            .split_whitespace()
            .last()
            .filter(|part| part.starts_with('@'))
            .map(str::to_string)
        {
            username = handle.clone();
            if let Some(stripped) = name.strip_suffix(&handle) {
                name = stripped.trim_end().to_string();
            }
        }

        name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() {
            name = raw.clone();
        }

        Self {
            raw,
            name,
            email,
            username,
        }
    }
}

#[derive(Debug)]
struct Proposal {
    kind: ProposalKind,
    number: u32,
    source_path: PathBuf,
    source_stem: String,
    title: String,
    description: String,
    status: String,
    status_key: String,
    authors: Vec<Author>,
    facets: Vec<String>,
    created: String,
    updated: String,
    modified: Option<toml::value::Datetime>,
    requirements: Vec<Requirement>,
    body: String,
    auxiliary_root: Option<PathBuf>,
    companions: Vec<Companion>,
}

impl Proposal {
    fn load(kind: ProposalKind, number: u32, source_path: PathBuf) -> Result<Self> {
        let source = fs::read_to_string(&source_path)
            .with_context(|| format!("could not read {}", source_path.display()))?;
        let metadata = MetadataBlock::parse(&source, &source_path)?;
        let declared_number = metadata
            .required(&[kind.prefix()], &source_path)?
            .parse::<u32>()
            .with_context(|| {
                format!(
                    "invalid {} number in {}",
                    kind.prefix(),
                    source_path.display()
                )
            })?;
        if declared_number != number {
            bail!(
                "{} declares {} {} but its filename indicates {}",
                source_path.display(),
                kind.prefix(),
                declared_number,
                number
            );
        }

        let title = metadata.required(&["Title"], &source_path)?;
        let status = metadata.required(&["Status"], &source_path)?;
        let status = if status.eq_ignore_ascii_case("Rejected (see below)") {
            "Rejected".to_string()
        } else {
            status
        };
        let authors = metadata
            .required(&["Authors", "Author", "Owner"], &source_path)?
            .split(',')
            .map(str::trim)
            .filter(|author| !author.is_empty())
            .map(Author::parse)
            .collect::<Vec<_>>();
        let facets = match kind {
            ProposalKind::Kip => metadata
                .optional(&["Layer"])
                .into_iter()
                .collect::<Vec<_>>(),
            ProposalKind::Kcc => [
                metadata.optional(&["Type"]),
                metadata.optional(&["Category"]),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>(),
        }
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|facet| !facet.is_empty())
                .map(|facet| {
                    if kind == ProposalKind::Kip
                        && facet.eq_ignore_ascii_case("Consensus (hard fork)")
                    {
                        "Consensus".to_string()
                    } else {
                        facet.to_string()
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect();
        let requirements = metadata
            .optional(&["Requires"])
            .into_iter()
            .flat_map(|value| {
                value
                    .split(',')
                    .filter_map(Requirement::parse)
                    .collect::<Vec<_>>()
            })
            .collect();
        let source_stem = source_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .context("proposal filename is not valid UTF-8")?
            .to_string();
        let auxiliary_root = source_path
            .parent()
            .map(|parent| parent.join(&source_stem))
            .filter(|path| path.is_dir());
        let companions = if let Some(root) = &auxiliary_root {
            Companion::load_all(root)?
        } else {
            Vec::new()
        };

        Ok(Self {
            kind,
            number,
            source_path,
            source_stem,
            description: metadata.optional(&["Description"]).unwrap_or_default(),
            status_key: Self::status_key(&status).to_string(),
            status,
            title,
            authors,
            facets,
            created: metadata.optional(&["Created"]).unwrap_or_default(),
            updated: metadata.optional(&["Updated"]).unwrap_or_default(),
            modified: None,
            requirements,
            body: metadata.body,
            auxiliary_root,
            companions,
        })
    }

    fn status_key(status: &str) -> &'static str {
        let status = status.to_ascii_lowercase();
        if status.contains("reject") || status.contains("withdraw") {
            "closed"
        } else if status.contains("implement")
            || status.contains("accept")
            || status.contains("final")
            || status.contains("active")
        {
            "active"
        } else if status.contains("last call") || status.contains("propos") {
            "review"
        } else {
            "draft"
        }
    }

    fn front_matter(
        &self,
        previous: Option<&Proposal>,
        next: Option<&Proposal>,
        companion: Option<&Companion>,
        seo: &SeoCatalog,
    ) -> PageFrontMatter {
        let is_companion = companion.is_some();
        let main_path = seo.path(self.kind, self.number);
        let description = if let Some(companion) = companion {
            seo.0
                .get(&format!("{}/{}", self.source_stem, companion.route))
                .map(|entry| entry.description.clone())
                .unwrap_or_else(|| {
                    MetadataBlock::summary(
                        &companion.body,
                        &format!(
                            "Kaspa {} {} companion: {}.",
                            self.kind.prefix(),
                            self.number,
                            companion.title
                        ),
                    )
                })
        } else if !self.description.is_empty() {
            self.description.clone()
        } else {
            seo.0.get(&self.source_stem)
                .map(|entry| entry.description.clone())
                .unwrap_or_else(|| {
                    MetadataBlock::summary(
                        &self.body,
                        &format!("Read Kaspa {} {}: {}, with proposal status and the original specification.", self.kind.prefix(), self.number, self.title),
                    )
                })
        };
        let (short_title, path, source_url, parent_url, parent_title) =
            if let Some(companion) = companion {
                (
                    companion.title.clone(),
                    format!("{}{}", main_path.trim_start_matches('/'), companion.route),
                    self.kind.source_url(&format!(
                        "{}/{}",
                        self.source_stem,
                        Self::web_path(&companion.relative)
                    )),
                    main_path.clone(),
                    self.title.clone(),
                )
            } else {
                (
                    self.title.clone(),
                    main_path.trim_matches('/').to_string(),
                    self.kind.source_url(
                        self.source_path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or_default(),
                    ),
                    String::new(),
                    String::new(),
                )
            };
        let requirement_labels = self
            .requirements
            .iter()
            .map(|requirement| requirement.label.clone())
            .collect();
        let requirement_urls = self
            .requirements
            .iter()
            .map(|requirement| requirement.url(self.kind, seo))
            .collect();

        PageFrontMatter {
            title: format!("{} {}: {}", self.kind.prefix(), self.number, short_title),
            description,
            template: "proposal.html",
            path,
            aliases: if let Some(companion) = companion {
                if main_path == format!("/{}/", self.number) {
                    Vec::new()
                } else {
                    vec![format!("/{}/{}/", self.number, companion.route)]
                }
            } else {
                let mut aliases = vec![format!("/{}/", self.source_stem)];
                if main_path != format!("/{}/", self.number) {
                    aliases.push(format!("/{}/", self.number));
                }
                aliases
            },
            date: if is_companion {
                None
            } else {
                MetadataBlock::date(&self.created)
            },
            updated: companion.map_or(self.modified, |document| document.modified),
            weight: if is_companion {
                10_000 + self.number
            } else {
                self.number
            },
            extra: PageExtra {
                kind: self.kind.prefix().to_string(),
                number: self.number,
                label: format!("{} {}", self.kind.prefix(), self.number),
                short_title,
                status: self.status.clone(),
                status_key: self.status_key.clone(),
                authors: self.authors.clone(),
                facets: self.facets.clone(),
                created: if is_companion {
                    String::new()
                } else {
                    self.created.clone()
                },
                updated: companion.map_or_else(
                    || self.updated.clone(),
                    |document| {
                        document
                            .modified
                            .map(|date| date.to_string())
                            .unwrap_or_default()
                    },
                ),
                source_url,
                requirement_labels,
                requirement_urls,
                is_companion,
                parent_url,
                parent_title,
                previous_url: previous
                    .map(|proposal| seo.path(proposal.kind, proposal.number))
                    .unwrap_or_default(),
                previous_label: previous
                    .map(|proposal| format!("{} {}", proposal.kind.prefix(), proposal.number))
                    .unwrap_or_default(),
                previous_title: previous
                    .map(|proposal| proposal.title.clone())
                    .unwrap_or_default(),
                next_url: next
                    .map(|proposal| seo.path(proposal.kind, proposal.number))
                    .unwrap_or_default(),
                next_label: next
                    .map(|proposal| format!("{} {}", proposal.kind.prefix(), proposal.number))
                    .unwrap_or_default(),
                next_title: next
                    .map(|proposal| proposal.title.clone())
                    .unwrap_or_default(),
            },
        }
    }

    fn resolve_auxiliary_target(
        &self,
        target: &str,
        companion: Option<&Companion>,
    ) -> Option<PathBuf> {
        self.auxiliary_root.as_ref()?;
        let raw_target = if companion.is_none() {
            target.strip_prefix(&format!("{}/", self.source_stem))?
        } else {
            target
        };
        let mut resolved = companion
            .and_then(|item| item.relative.parent())
            .map(Path::to_path_buf)
            .unwrap_or_default();

        for component in Path::new(raw_target).components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    resolved.pop();
                }
                Component::Normal(part) => resolved.push(part),
                Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        Some(resolved)
    }

    fn web_path(path: &Path) -> String {
        path.components()
            .filter_map(|component| match component {
                Component::Normal(part) => Some(part.to_string_lossy()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/")
            .replace(' ', "%20")
    }
}

#[derive(Debug)]
struct Companion {
    relative: PathBuf,
    route: String,
    title: String,
    body: String,
    modified: Option<toml::value::Datetime>,
}

impl Companion {
    fn load_all(root: &Path) -> Result<Vec<Self>> {
        let mut companions = Vec::new();
        for entry in WalkDir::new(root).follow_links(false) {
            let entry = entry?;
            if !entry.file_type().is_file()
                || entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    != Some("md")
            {
                continue;
            }
            let relative = entry.path().strip_prefix(root)?.to_path_buf();
            let body = fs::read_to_string(entry.path())
                .with_context(|| format!("could not read {}", entry.path().display()))?;
            let title = body
                .lines()
                .find_map(|line| line.strip_prefix("# "))
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    relative
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or("Companion document")
                        .replace(['-', '_'], " ")
                });
            companions.push(Self {
                route: Proposal::web_path(&relative.with_extension("")),
                relative,
                title,
                body,
                modified: None,
            });
        }
        companions.sort_by(|left, right| left.route.cmp(&right.route));
        Ok(companions)
    }
}

#[derive(Debug)]
struct Requirement {
    kind: ProposalKind,
    number: u32,
    label: String,
}

impl Requirement {
    fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().to_ascii_uppercase().replace(' ', "-");
        let (prefix, number) = normalized.split_once('-')?;
        let kind = ProposalKind::from_prefix(prefix)?;
        let number = number.parse::<u32>().ok()?;
        Some(Self {
            kind,
            number,
            label: format!("{} {}", kind.prefix(), number),
        })
    }

    fn url(&self, current_kind: ProposalKind, seo: &SeoCatalog) -> String {
        let path = seo.path(self.kind, self.number);
        if self.kind == current_kind {
            path
        } else {
            format!("https://{}{path}", self.kind.domain())
        }
    }
}

#[derive(Serialize)]
struct SectionFrontMatter {
    title: String,
    description: String,
    template: &'static str,
    sort_by: &'static str,
    extra: SectionExtra,
}

#[derive(Serialize)]
struct SectionExtra {
    proposal_count: usize,
    statuses: Vec<String>,
    facets: Vec<String>,
}

#[derive(Serialize)]
struct PageFrontMatter {
    title: String,
    description: String,
    template: &'static str,
    path: String,
    aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    date: Option<toml::value::Datetime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated: Option<toml::value::Datetime>,
    weight: u32,
    extra: PageExtra,
}

#[derive(Serialize)]
struct PageExtra {
    kind: String,
    number: u32,
    label: String,
    short_title: String,
    status: String,
    status_key: String,
    authors: Vec<Author>,
    facets: Vec<String>,
    created: String,
    updated: String,
    source_url: String,
    requirement_labels: Vec<String>,
    requirement_urls: Vec<String>,
    is_companion: bool,
    parent_url: String,
    parent_title: String,
    previous_url: String,
    previous_label: String,
    previous_title: String,
    next_url: String,
    next_label: String,
    next_title: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generator() -> SiteGenerator {
        SiteGenerator {
            config: GeneratorConfig {
                kips: ProposalSource::Directory(PathBuf::from(".sources/kips")),
                kccs: ProposalSource::Directory(PathBuf::from(".sources/kccs")),
                output: PathBuf::from(".generated"),
                shared_site: PathBuf::from("site"),
            },
            seo: SeoCatalog::parse(include_str!("../site/seo.toml")).unwrap(),
            markdown_link: Regex::new(r"(?P<open>!?\[[^\]]*\]\()(?P<target><[^>\n]+>|[^)\s\n]+)(?P<close>\))").unwrap(),
            bare_url: Regex::new(BARE_URL_PATTERN).unwrap(),
            proposal_target: Regex::new(r"(?i)(?:^|/)(kip|kcc)-0*([0-9]+)\.md$").unwrap(),
            internal_anchor_link: Regex::new(r"\]\(#([A-Za-z0-9_-]+)\)").unwrap(),
            reference_anchor: Regex::new(r#"<a id="ref-[0-9]+"></a>\[([0-9]+)\]"#).unwrap(),
            plain_proposal_reference: Regex::new(r"(?mi)^(?P<prefix>\s*\[[0-9]+\]\s+)(?P<kind>KIP|KCC)-0*(?P<number>[0-9]+)(?P<suffix>:)").unwrap(),
        }
    }

    #[test]
    fn discovery_persists_new_routes_before_linking_and_preserves_existing_entries() {
        let root = env::temp_dir().join(format!("kaspa-seo-discovery-test-{}", std::process::id()));
        for directory in ["kips/kip-9999", "kccs", "site/templates", "site/static"] {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        let original = "# Keep this curated entry and comment.\n[kip-9998]\nslug = 'kip-9998-original-name'\ndescription = 'An existing curated description for this proposal.'\n";
        let registry = root.join("site/seo.toml");
        fs::write(&registry, original).unwrap();
        for (file, kind, number, title, body) in [
            ("kips/kip-9998.md", "KIP", 9998, "Changed title", "Body"),
            (
                "kips/kip-9999.md",
                "KIP",
                9999,
                "New Tokens: V2 / API!",
                "[KCC](https://github.com/kaspanet/kccs/blob/main/kcc-9999.md)",
            ),
            (
                "kccs/kcc-9999.md",
                "KCC",
                9999,
                "New Tokens: V2 / API!",
                "[KIP](https://github.com/kaspanet/kips/blob/master/kip-9999.md)",
            ),
            ("kccs/kcc-9998.md", "KCC", 9998, "新提案", "Body"),
        ] {
            fs::write(root.join(file), format!("```\n{kind}: {number}\nTitle: {title}\nStatus: Draft\nAuthors: Example\n```\n\n{body}")).unwrap();
        }
        fs::write(
            root.join("kips/kip-9999/notes.md"),
            "# Notes\n\nCompanion body.",
        )
        .unwrap();
        let mut generator = generator();
        generator.config.kips = ProposalSource::Directory(root.join("kips"));
        generator.config.kccs = ProposalSource::Directory(root.join("kccs"));
        generator.config.shared_site = root.join("site");
        generator.config.output = root.join(".generated");
        generator.seo = SeoCatalog::parse(original).unwrap();
        generator.run().unwrap();

        let saved = fs::read_to_string(&registry).unwrap();
        assert!(saved.starts_with(original));
        let catalog = SeoCatalog::parse(&saved).unwrap();
        assert_eq!(
            catalog.path(ProposalKind::Kip, 9998),
            "/kip-9998-original-name/"
        );
        assert_eq!(
            catalog.path(ProposalKind::Kip, 9999),
            "/kip-9999-new-tokens-v2-api/"
        );
        assert_eq!(
            catalog.path(ProposalKind::Kcc, 9999),
            "/kcc-9999-new-tokens-v2-api/"
        );
        assert_eq!(catalog.path(ProposalKind::Kcc, 9998), "/kcc-9998-proposal/");
        assert!(
            catalog.0["kip-9999"]
                .description
                .contains("New Tokens: V2 / API!")
        );
        assert!(catalog.0.contains_key("kip-9999/notes"));
        let kip = fs::read_to_string(root.join(".generated/kips/content/9999-main.md")).unwrap();
        let kcc = fs::read_to_string(root.join(".generated/kccs/content/9999-main.md")).unwrap();
        assert!(kip.contains("https://kccs.dev/kcc-9999-new-tokens-v2-api/"));
        assert!(kcc.contains("https://kips.dev/kip-9999-new-tokens-v2-api/"));
        let redirects = fs::read_to_string(root.join(".generated/kips/static/_redirects")).unwrap();
        assert!(redirects.contains("/9999/ /kip-9999-new-tokens-v2-api/ 301"));

        let proposal_path = root.join("kips/kip-9999.md");
        let changed = fs::read_to_string(&proposal_path)
            .unwrap()
            .replace("New Tokens: V2 / API!", "Renamed proposal");
        fs::write(proposal_path, changed).unwrap();
        generator.seo = catalog;
        generator.run().unwrap();
        assert_eq!(fs::read_to_string(&registry).unwrap(), saved);
        fs::remove_file(&registry).unwrap();
        generator.seo = SeoCatalog::parse("").unwrap();
        generator.run().unwrap();
        let regenerated = SeoCatalog::parse(&fs::read_to_string(&registry).unwrap()).unwrap();
        assert_eq!(
            regenerated.path(ProposalKind::Kip, 9999),
            "/kip-9999-renamed-proposal/"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn published_routes_are_stable_and_unknown_numbers_remain_numeric() {
        let seo = SeoCatalog::parse(include_str!("../site/seo.toml")).unwrap();
        assert_eq!(
            seo.path(ProposalKind::Kcc, 20),
            "/kcc-20-fungible-token-covenant/"
        );
        assert_eq!(seo.path(ProposalKind::Kip, 9999), "/9999/");
        assert!(
            SeoCatalog::parse("[kip-0017]\nslug = '../escape'\ndescription = 'Summary'").is_err()
        );
        assert!(
            SeoCatalog::parse("[kip-0017]\nslug = 'kcc-17-wrong-domain'\ndescription = 'Summary'")
                .is_err()
        );
        assert!(
            SeoCatalog::parse("[kip-0017]\nslug = 'kip-18-wrong-number'\ndescription = 'Summary'")
                .is_err()
        );
    }

    #[test]
    fn dates_reject_invalid_calendar_values_and_timestamps() {
        assert!(MetadataBlock::date("2024-02-29").is_some());
        for invalid in [
            "",
            "2025-02-29",
            "2026-04-31",
            "2026-13-01",
            "2026-09-10T00:00:00Z",
            "unknown",
        ] {
            assert!(MetadataBlock::date(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn descriptions_skip_code_and_metadata_and_truncate_at_word_boundaries() {
        assert_eq!(
            MetadataBlock::summary(
                "# Title\n\n```rust\nsecret example\n```\n\nStatus: Draft\n\nUseful prose here.",
                "Fallback"
            ),
            "Useful prose here."
        );
        assert_eq!(
            MetadataBlock::summary("<script>bad</script>\n\n", "Fallback"),
            "Fallback"
        );
        assert_eq!(
            MetadataBlock::summary("[Linked](file.md) text.", "Fallback"),
            "Fallback"
        );
        let summary = MetadataBlock::summary(&"Kaspa ünicode words ".repeat(30), "Fallback");
        assert!(summary.chars().count() <= 181);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn heading_hierarchy_preserves_anchors_and_fenced_examples() {
        let generator = generator();
        let source = "# Intro\n## Detail {#custom}\n# Intro\n````md\n# Not a heading\n```\n# Still code\n````\n## Finish";
        let result = generator.normalize_heading_ids(source);
        assert!(result.starts_with("## Intro {#intro}\n### Detail {#custom}\n## Intro {#intro-1}"));
        assert!(result.contains("````md\n# Not a heading\n```\n# Still code\n````"));
        assert!(result.ends_with("### Finish {#finish}"));
        assert_eq!(
            generator.normalize_heading_ids("## Abstract\nText"),
            "## Abstract {#abstract}\nText"
        );
    }

    #[test]
    fn migration_rewrites_cross_site_companion_and_asset_links() {
        let generator = generator();
        let proposal = Proposal {
            kind: ProposalKind::Kip,
            number: 21,
            source_path: PathBuf::from("kip-0021.md"),
            source_stem: "kip-0021".into(),
            title: "A mutable title".into(),
            description: String::new(),
            status: "Active".into(),
            status_key: "active".into(),
            authors: Vec::new(),
            facets: Vec::new(),
            created: "2026-02-17".into(),
            updated: "2026-05-28".into(),
            modified: MetadataBlock::date("2026-08-01"),
            requirements: Vec::new(),
            body: String::new(),
            auxiliary_root: Some(PathBuf::from("kip-0021")),
            companions: Vec::new(),
        };
        let source = "[KCC](https://github.com/kaspanet/kccs/blob/main/kcc-0020.md#1-state)\n[Spec](kip-0021/impl-spec.md#intro)\n[Asset](kip-0021/vector.json)\n[License](LICENSE.md#terms)";
        let rewritten = generator.rewrite_links(&proposal, source, None);
        assert!(rewritten.contains("https://kccs.dev/kcc-20-fungible-token-covenant/#1-state"));
        assert!(rewritten.contains("/kip-21-partitioned-sequencing/impl-spec/#intro"));
        assert!(rewritten.contains("/21/vector.json"));
        assert!(rewritten.contains(&proposal.kind.source_url("LICENSE.md#terms")));
        let main = proposal.front_matter(None, None, None, &generator.seo);
        assert_eq!(main.path, "kip-21-partitioned-sequencing");
        assert!(main.aliases.contains(&"/21/".into()));
        let companion = Companion {
            relative: PathBuf::from("impl-spec.md"),
            route: "impl-spec".into(),
            title: "Implementation".into(),
            body: "Useful implementation notes.".into(),
            modified: None,
        };
        let page = proposal.front_matter(None, None, Some(&companion), &generator.seo);
        assert_eq!(page.path, "kip-21-partitioned-sequencing/impl-spec");
        assert_eq!(page.aliases, ["/21/impl-spec/"]);
        assert_eq!(page.extra.parent_url, "/kip-21-partitioned-sequencing/");
        assert!(page.date.is_none());
        assert!(page.updated.is_none());
        assert!(page.extra.created.is_empty());
        assert_ne!(main.description, page.description);
    }

    #[test]
    fn metadata_block_separates_headers_from_content() {
        let parsed = MetadataBlock::parse(
            "```\nKIP: 7\nTitle: Example\n```\n\n## Abstract\nBody",
            Path::new("kip-0007.md"),
        )
        .unwrap();

        assert_eq!(parsed.fields["KIP"], "7");
        assert_eq!(parsed.body, "## Abstract\nBody");
    }

    #[test]
    fn requirement_links_switch_domains_when_needed() {
        let requirement = Requirement::parse("KIP-0020").unwrap();
        assert_eq!(requirement.label, "KIP 20");
        let seo = SeoCatalog::parse(include_str!("../site/seo.toml")).unwrap();
        assert_eq!(
            requirement.url(ProposalKind::Kcc, &seo),
            "https://kips.dev/kip-20-covenant-ids/"
        );
        assert_eq!(
            Requirement::parse("KCC-0000")
                .unwrap()
                .url(ProposalKind::Kcc, &seo),
            "/kcc-0-purpose-and-guidelines/"
        );
    }

    #[test]
    fn author_metadata_separates_names_from_contact_details() {
        let author = Author::parse("Example Åuthor (@example_author) <author@example.com>");
        assert_eq!(author.name, "Example Åuthor");
        assert_eq!(author.username, "@example_author");
        assert_eq!(author.email, "author@example.com");

        let author = Author::parse("Sample Writer @sample_writer");
        assert_eq!(author.name, "Sample Writer");
        assert_eq!(author.username, "@sample_writer");
        assert!(author.email.is_empty());
    }

    #[test]
    fn proposal_load_normalizes_presentation_metadata() {
        let source_path = env::temp_dir().join(format!(
            "kaspa-proposals-site-{}-kip-9999.md",
            std::process::id()
        ));
        fs::write(
            &source_path,
            "```\nKIP: 9999\nTitle: Example\nStatus: Rejected (see below)\nAuthors: Example\nLayer: Consensus (hard fork), API/RPC\n```\n\nBody",
        )
        .unwrap();

        let proposal = Proposal::load(ProposalKind::Kip, 9999, source_path.clone());
        fs::remove_file(source_path).unwrap();
        let proposal = proposal.unwrap();

        assert_eq!(proposal.status, "Rejected");
        assert_eq!(proposal.facets, ["Consensus", "API/RPC"]);
    }

    #[test]
    fn bare_urls_become_autolinks_without_changing_markdown_links() {
        let bare_url = Regex::new(BARE_URL_PATTERN).unwrap();
        let source = "[1] Bare https://example.com/reference.\n\
                      [2] Existing [link](https://example.com/existing)";

        assert_eq!(
            bare_url.replace_all(source, BARE_URL_REPLACEMENT),
            "[1] Bare <https://example.com/reference>.\n\
             [2] Existing [link](https://example.com/existing)"
        );
    }
}
