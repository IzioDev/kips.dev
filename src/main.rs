use anyhow::{Context, Result, bail};
use regex::{Captures, Regex};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use walkdir::WalkDir;

const BARE_URL_PATTERN: &str =
    r"(?m)(?P<prefix>^|[ \t])(?P<url>https?://[^\s<>()]*[A-Za-z0-9/#=_~+%-])";
const BARE_URL_REPLACEMENT: &str = "${prefix}<${url}>";

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
                "Technical proposals for Kaspa consensus, node behavior, network upgrades, and client APIs."
            }
            Self::Kcc => {
                "Shared conventions for interoperable applications, assets, wallets, covenants, and ecosystem tooling."
            }
        }
    }

    fn facet_label(self) -> &'static str {
        match self {
            Self::Kip => "Layer",
            Self::Kcc => "Type / category",
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
    markdown_link: Regex,
    bare_url: Regex,
    proposal_target: Regex,
    internal_anchor_link: Regex,
    reference_anchor: Regex,
    plain_proposal_reference: Regex,
}

impl SiteGenerator {
    fn from_env() -> Result<Self> {
        Ok(Self {
            config: GeneratorConfig::from_env()?,
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

    fn run(&self) -> Result<()> {
        for kind in [ProposalKind::Kip, ProposalKind::Kcc] {
            self.config.source_for(kind).prepare(kind)?;
        }
        self.clean_output()?;

        for kind in [ProposalKind::Kip, ProposalKind::Kcc] {
            let proposals = self.load_proposals(kind)?;
            self.write_site(kind, &proposals)?;
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

            proposals.push(Proposal::load(kind, number, entry.path())?);
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

        for (index, proposal) in proposals.iter().enumerate() {
            self.copy_auxiliary_assets(proposal, &site_root.join("static"))?;

            let previous = index.checked_sub(1).map(|position| &proposals[position]);
            let next = proposals.get(index + 1);
            let body =
                self.normalize_heading_ids(&self.rewrite_links(proposal, &proposal.body, None));
            let front_matter = proposal.front_matter(previous, next, false, None);
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
                let front_matter = proposal.front_matter(None, None, true, Some(companion));
                let safe_name = companion.route.replace('/', "-");
                fs::write(
                    content_root.join(format!("{:04}-companion-{}.md", proposal.number, safe_name)),
                    format!("+++\n{}+++\n\n{}\n", toml::to_string(&front_matter)?, body),
                )?;
            }
        }

        Ok(())
    }

    fn site_config(&self, kind: ProposalKind) -> String {
        let counterpart = kind.counterpart();
        format!(
            r#"base_url = "https://{}"
title = "{}.dev — {}"
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
            kind.plural(),
            kind.long_name(),
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
                        "{base}/{number}/{}",
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
                            "/{}/{}/{}",
                            proposal.number,
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
                    "{}[{}-{number}]({base}/{number}/){}",
                    &captures["prefix"],
                    target_kind.prefix(),
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
        let mut in_fence = false;
        let mut seen = BTreeMap::<String, usize>::new();
        let mut output = Vec::new();

        for line in body.lines() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
                in_fence = !in_fence;
                output.push(line.to_string());
                continue;
            }
            if in_fence || line.contains("{#") {
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
            output.push(format!("{line} {{#{unique_slug}}}"));
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
            description: metadata
                .optional(&["Description"])
                .unwrap_or_else(|| format!("{} {}: {title}", kind.prefix(), number)),
            status_key: Self::status_key(&status).to_string(),
            status,
            title,
            authors,
            facets,
            created: metadata.optional(&["Created"]).unwrap_or_default(),
            updated: metadata.optional(&["Updated"]).unwrap_or_default(),
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
        is_companion: bool,
        companion: Option<&Companion>,
    ) -> PageFrontMatter {
        let (short_title, description, path, source_url, parent_url, parent_title) =
            if let Some(companion) = companion {
                (
                    companion.title.clone(),
                    format!(
                        "Companion document for {} {}: {}",
                        self.kind.prefix(),
                        self.number,
                        self.title
                    ),
                    format!("{}/{}", self.number, companion.route),
                    self.kind.source_url(&format!(
                        "{}/{}",
                        self.source_stem,
                        Self::web_path(&companion.relative)
                    )),
                    format!("/{}/", self.number),
                    self.title.clone(),
                )
            } else {
                (
                    self.title.clone(),
                    self.description.clone(),
                    self.number.to_string(),
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
            .map(|requirement| requirement.url(self.kind))
            .collect();

        PageFrontMatter {
            title: format!("{} {}: {}", self.kind.prefix(), self.number, short_title),
            description,
            template: "proposal.html",
            path,
            aliases: if is_companion {
                Vec::new()
            } else {
                vec![format!("/{}/", self.source_stem)]
            },
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
                created: self.created.clone(),
                updated: self.updated.clone(),
                source_url,
                requirement_labels,
                requirement_urls,
                is_companion,
                parent_url,
                parent_title,
                previous_url: previous
                    .map(|proposal| format!("/{}/", proposal.number))
                    .unwrap_or_default(),
                previous_label: previous
                    .map(|proposal| format!("{} {}", proposal.kind.prefix(), proposal.number))
                    .unwrap_or_default(),
                previous_title: previous
                    .map(|proposal| proposal.title.clone())
                    .unwrap_or_default(),
                next_url: next
                    .map(|proposal| format!("/{}/", proposal.number))
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
        let number = number.trim_start_matches('0').parse::<u32>().ok()?;
        Some(Self {
            kind,
            number,
            label: format!("{} {}", kind.prefix(), number),
        })
    }

    fn url(&self, current_kind: ProposalKind) -> String {
        if self.kind == current_kind {
            format!("/{}/", self.number)
        } else {
            format!("https://{}/{}/", self.kind.domain(), self.number)
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
        assert_eq!(requirement.url(ProposalKind::Kcc), "https://kips.dev/20/");
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
