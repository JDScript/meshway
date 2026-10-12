use crate::object::wire_object;
use chrono::{DateTime, Utc};
use meshway_model::{RequestId, Revision};
use std::collections::BTreeMap;

wire_object! { #[derive(Clone, Debug)]
/// Transient validated rule-set content produced by a noncommitting configuration preview.
pub struct PreparedRuleSet {
    /// `sha256:` and the lowercase hex SHA-256 of the complete artifact; content addressed, no ID is reserved.
    pub digest: String [pattern = "^sha256:[0-9a-f]{64}$"],
    /// Artifact length in bytes.
    pub bytes: u32,
    /// Canonical members: names, keywords and merged address ranges.
    pub entries: u32,
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)]
/// Immutable rule-set artifact selected by a resource binding. Artifacts are content addressed and shared by every network.
pub struct ArtifactSummary {
    /// `sha256:` and the lowercase hex SHA-256 of the complete artifact; Nodes fetch it by this digest.
    pub digest: String [pattern = "^sha256:[0-9a-f]{64}$"],
    /// `MWRS` major format version.
    pub format: u16,
    /// Artifact length in bytes.
    pub bytes: u32,
    /// Canonical members: names, keywords and merged address ranges.
    pub entries: u32,
    /// UTC time the Server first stored this content, distinct from later refresh checks.
    pub created_at: DateTime<Utc>,
} }
meshway_model::string_enum! {
/// Stable failure category for rule-set preparation or refresh.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ResourceProblem {
    /// DNS resolution, HTTPS transfer or its deadline failed to provide the source.
    DownloadFailed,
    /// The URL or resolved destination violates the allowed HTTPS public-source constraints, or the document is not valid in its declared format.
    InvalidSource,
    /// Source content exceeds the bounded download size.
    TooLarge,
    /// No entry of the source could be converted.
    ConversionFailed,
    ConfigurationChanged,
    /// The process-wide download or preparation capacity is exhausted.
    BudgetExceeded,
    /// A remote or catalog rule set has no validated content bound for use yet.
    NotReady,
    /// Prepared contents fail validation against the complete current configuration and its permissions.
    ValidationFailed,
    /// The converted set exceeds an artifact limit (names, keywords, ranges or 16 MiB).
    RuleSetBudget,
    /// The catalog signature is missing, malformed, by an untrusted key or does not verify.
    SignatureInvalid,
    /// The catalog's sequence is lower than one this Server already accepted.
    CatalogRegressed,
    /// The catalog has no set of this name.
    UnknownRuleSet,
    /// An artifact differs from its catalog entry or is not a valid artifact.
    IntegrityFailed,
}
}
wire_object! {
/// Idempotent manual refresh of one remote or catalog rule set.
pub struct RefreshRuleSetRequest {
    /// Key retained for 24 hours per user and network; same-key retries return the current view, while reuse for another key conflicts.
    pub request_id: RequestId
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)]
/// Accepted source entries by the member kind they became, counted before duplicates and covered members are dropped.
pub struct RuleSetConverted {
    pub exact: u32,
    pub suffix: u32,
    pub subdomain: u32,
    pub keyword: u32,
    pub cidr: u32,
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)]
/// How the bound content was converted from its source.
pub struct RuleSetConversion {
    /// Accepted entries by kind.
    pub converted: RuleSetConverted,
    /// Accepted names and keywords dropped as duplicates or covered by another member.
    pub redundant: u32,
    /// Skipped entries by reason (`regex`, `invalidName`, `classical:<TYPE>`, `unsupportedField:<name>`, ...); the content is a subset of the source's semantics whenever this is not empty.
    pub unconverted: BTreeMap<String, u32>,
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)]
/// The catalog a provider-referenced set came from.
pub struct RuleSetProviderView {
    /// Provider name, the part of the key before `:`.
    pub name: String,
    /// Catalog `index.json` URL.
    pub catalog: String,
    /// Catalog collection of the provider.
    pub collection: String,
    /// Sequence of the verified catalog the bound artifact was taken from; null before any content is bound.
    #[serde(deserialize_with = "crate::object::required_nullable")] pub sequence: Option<u32> [required = true],
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)]
/// Where the bound content's source came from.
pub struct RuleSetSourceView {
    /// Source format the content was converted from.
    pub format: String,
    /// Upstream repository of a catalog set; null for a remote URL or inline set.
    #[serde(deserialize_with = "crate::object::required_nullable")] pub repo: Option<String> [required = true],
    /// Upstream branch or tag; null when not from a catalog.
    #[serde(deserialize_with = "crate::object::required_nullable")] #[serde(rename = "ref")] pub reference: Option<String> [required = true],
    /// Upstream commit; null when not from a catalog.
    #[serde(deserialize_with = "crate::object::required_nullable")] pub commit: Option<String> [required = true],
    /// Upstream file path; null when not from a catalog.
    #[serde(deserialize_with = "crate::object::required_nullable")] pub path: Option<String> [required = true],
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)]
/// The catalog still publishes an older artifact for this set.
pub struct RuleSetHeld {
    /// UTC time since which the catalog holds the previous artifact.
    pub since: DateTime<Utc>,
    /// The publisher's reason.
    pub reason: String,
} }
wire_object! { #[derive(Clone, Debug)]
/// Current rule-set declaration, selected immutable artifact and actual check status.
pub struct RuleSetView {
    /// Rule-set key as routing rules reference it: a network `ruleSets` key, or `<provider>:<name>`.
    pub key: String,
    /// Current source revision owning this definition and resource view.
    pub configuration_revision: Revision,
    /// Whether the set comes from a `<provider>:<name>` reference rather than a `ruleSets` declaration.
    pub implicit: bool,
    /// Source declaration when included in the authorized resource scope; null for implicit sets or when the scope does not expose it.
    pub definition: Option<meshway_configuration::RuleSet>,
    /// Catalog of an implicit set; null for declared sets.
    #[serde(deserialize_with = "crate::object::required_nullable")] pub provider: Option<RuleSetProviderView> [required = true],
    /// Selected validated artifact, retained after failed refreshes; null when no content is bound.
    pub artifact: Option<ArtifactSummary>,
    /// Provenance of the bound content; null when none is bound.
    #[serde(deserialize_with = "crate::object::required_nullable")] pub source: Option<RuleSetSourceView> [required = true],
    /// Conversion summary of the bound content; null when none is bound. The full skipped-entry list is `GET .../rule-sets/{ruleSetKey}/report`.
    #[serde(deserialize_with = "crate::object::required_nullable")] pub conversion: Option<RuleSetConversion> [required = true],
    /// The catalog holds an older artifact for this set; null otherwise.
    #[serde(deserialize_with = "crate::object::required_nullable")] pub held: Option<RuleSetHeld> [required = true],
    /// Last check failure or `not_ready` for missing initial remote content; null when no problem is recorded.
    pub problem: Option<ResourceProblem>,
    /// UTC time of the latest persisted check, successful or failed; null when no check record exists.
    pub checked_at: Option<DateTime<Utc>>,
    /// Last successful download/conversion check, not artifact creation.
    pub last_success_at: Option<DateTime<Utc>>,
} }
wire_object! { #[derive(Clone, Debug)]
/// Authorized resource views for all rule sets or one requested key.
pub struct RuleSetList {
    /// Current definitions and check results, restricted to the selected key for a detail request.
    pub items: Vec<RuleSetView>
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)]
/// One source entry that did not become a member.
pub struct RuleSetSkippedEntry {
    /// Position in the source: `/rules/3/domain_regex/0`, `/payload/7`, `line 12` or `/domainSuffix/2`.
    pub at: String,
    /// Skip reason, as in `RuleSetConversion.unconverted`.
    pub reason: String,
    /// The entry text, truncated to 256 characters.
    pub entry: String,
} }
wire_object! { #[derive(Clone, Debug)]
/// Complete conversion report of the bound content.
pub struct RuleSetReport {
    pub key: String,
    /// The bound artifact the report describes.
    pub digest: String [pattern = "^sha256:[0-9a-f]{64}$"],
    pub conversion: RuleSetConversion,
    /// The first 1,000 skipped entries; the rest are only counted in `conversion`. Empty when the provider published only a summary.
    pub items: Vec<RuleSetSkippedEntry>,
} }

/// Query of `listRuleSetCatalog`: URI fields, not a JSON body.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct RuleSetCatalogQuery {
    /// Provider name effective in this network: a built-in provider or a `ruleSetProviders` key.
    #[param(min_length = 1, max_length = 32)]
    pub provider: String,
    /// Case-insensitive prefix of the set name (`open` finds `openai`); omission or empty lists every set.
    #[param(max_length = 128)]
    pub q: Option<String>,
    /// Maximum number of sets to return, from 1 through 200; default 100.
    #[serde(default, deserialize_with = "crate::networks::optional_limit")]
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<u16>,
    /// Opaque continuation cursor from the preceding page with the same `provider` and `q`, limited to 4096 characters.
    #[serde(default, deserialize_with = "crate::networks::optional_cursor")]
    #[param(max_length = 4096)]
    pub cursor: Option<String>,
}
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)]
/// The verified catalog a provider's sets are listed from.
pub struct RuleSetCatalogProvider {
    /// Provider name, the part of a reference before `:`.
    pub name: String,
    /// Catalog `index.json` URL.
    pub catalog: String,
    /// Catalog collection of the provider.
    pub collection: String,
    /// Sequence of the newest catalog this Server accepted.
    pub sequence: u32,
    /// When the publisher generated that catalog.
    pub generated_at: DateTime<Utc>,
    /// When this Server last verified it.
    pub verified_at: DateTime<Utc>,
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)]
/// Upstream file a catalog set was converted from.
pub struct RuleSetUpstream {
    pub repo: String,
    #[serde(rename = "ref")] pub reference: String,
    pub commit: String,
    /// File path in the upstream repository.
    pub path: String,
} }
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)]
/// One set of a provider's verified catalog.
pub struct RuleSetCatalogSet {
    /// What routing rules write in `match.ruleSets`: `<provider>:<name>`.
    pub reference: String,
    /// Set name within the provider.
    pub name: String,
    /// Catalog set `<collection>/<name>`.
    pub set: String,
    /// Artifact the catalog publishes for this set.
    pub digest: String [pattern = "^sha256:[0-9a-f]{64}$"],
    /// Artifact length in bytes.
    pub bytes: u32,
    /// Canonical members: names, keywords and merged address ranges.
    pub entries: u32,
    pub upstream: RuleSetUpstream,
    /// The catalog still publishes an older artifact for this set; null otherwise.
    #[serde(deserialize_with = "crate::object::required_nullable")] pub held: Option<RuleSetHeld> [required = true],
} }
wire_object! { #[derive(Clone, Debug)]
/// A page of one provider's catalog sets, ordered by name.
pub struct RuleSetCatalogPage {
    pub provider: RuleSetCatalogProvider,
    pub items: Vec<RuleSetCatalogSet>,
    /// Sets matching `q` in this catalog.
    pub total: u32,
    /// Opaque continuation cursor, or null when no further page is available.
    #[serde(deserialize_with = "crate::object::required_nullable")] pub next_cursor: Option<String> [required = true],
} }
meshway_model::string_enum! {
/// Member kind a rule-set entry page lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum RuleSetEntryKind {
    /// Exact, suffix and subdomain names.
    Domain,
    Keyword,
    /// Merged IPv4 ranges.
    Ipv4,
    /// Merged IPv6 ranges.
    Ipv6,
}
}
/// Query of `listRuleSetEntries`: URI fields, not a JSON body.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct RuleSetEntriesQuery {
    /// Member kind to list.
    pub kind: RuleSetEntryKind,
    /// `domain`: names ending with `q` (`openai.com` lists it and every name below it; `.openai.com` only the names below), compared after lowercasing and removing one trailing dot. `keyword`: keywords starting with `q`. `ipv4`／`ipv6`: an address; the page starts at the range containing it or the first range after it. Omission or empty lists from the start.
    #[param(max_length = 253)]
    pub q: Option<String>,
    /// Maximum number of members to return, from 1 through 200; default 100.
    #[serde(default, deserialize_with = "crate::networks::optional_limit")]
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<u16>,
    /// Opaque continuation cursor from the preceding page with the same `kind` and `q`, limited to 4096 characters.
    #[serde(default, deserialize_with = "crate::networks::optional_cursor")]
    #[param(max_length = 4096)]
    pub cursor: Option<String>,
}
crate::object::wire_tagged! { tag="kind"; #[derive(Clone, Debug, PartialEq, Eq)]
/// One canonical member of a rule set.
pub enum RuleSetMember {
    /// Matches exactly this name.
    Exact { name: String },
    /// Matches the name and every name below it.
    Suffix { name: String },
    /// Matches only names below it.
    Subdomain { name: String },
    /// Matches every name containing it.
    Keyword { keyword: String },
    /// Matches every address from `first` through `last`, both included; adjacent and overlapping source CIDRs are merged.
    IpRange { first: String, last: String },
}
}
wire_object! { #[derive(Clone, Debug, PartialEq, Eq)]
/// Members of the bound content by kind; address ranges are merged.
pub struct RuleSetMemberCounts {
    pub exact: u32,
    pub suffix: u32,
    pub subdomain: u32,
    pub keyword: u32,
    pub ipv4: u32,
    pub ipv6: u32,
} }
wire_object! { #[derive(Clone, Debug)]
/// A page of one kind of members of a rule set's bound content.
pub struct RuleSetEntryPage {
    pub key: String,
    /// The bound artifact the page is read from; a cursor stays valid only while this binding is current.
    pub digest: String [pattern = "^sha256:[0-9a-f]{64}$"],
    pub kind: RuleSetEntryKind,
    /// Members of every kind in the bound content.
    pub counts: RuleSetMemberCounts,
    /// `domain`: ordered by reversed name (names sharing a suffix are adjacent); `keyword`: sorted; addresses: ascending.
    pub items: Vec<RuleSetMember>,
    /// Opaque continuation cursor, or null when no further page is available.
    #[serde(deserialize_with = "crate::object::required_nullable")] pub next_cursor: Option<String> [required = true],
} }
/// Query of `matchRuleSet`: exactly one of `name` and `ip`.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct RuleSetMatchQuery {
    /// DNS query name, judged as routing does: ASCII lowercase without one trailing dot, `_` allowed.
    #[param(max_length = 254)]
    pub name: Option<String>,
    /// IPv4 or IPv6 address.
    #[param(max_length = 64)]
    pub ip: Option<String>,
}
wire_object! { #[derive(Clone, Debug)]
/// Whether a rule set's bound content contains a name or address.
pub struct RuleSetMatchResponse {
    pub key: String,
    /// The bound artifact that was judged.
    pub digest: String [pattern = "^sha256:[0-9a-f]{64}$"],
    /// The query as judged: the normalized name or the address.
    pub query: String,
    /// The member that matched, or null when the set does not contain the query. Names try suffix and subdomain members from the shortest, then exact, then keywords.
    #[serde(deserialize_with = "crate::object::required_nullable")] pub member: Option<RuleSetMember> [required = true],
} }

impl From<meshway_rules::Counts> for RuleSetMemberCounts {
    fn from(c: meshway_rules::Counts) -> Self {
        Self {
            exact: c.exact,
            suffix: c.suffix,
            subdomain: c.subdomain,
            keyword: c.keyword,
            ipv4: c.ipv4,
            ipv6: c.ipv6,
        }
    }
}

impl From<meshway_rules::DomainEntry> for RuleSetMember {
    fn from(entry: meshway_rules::DomainEntry) -> Self {
        let name = entry.name;
        match entry.kind {
            meshway_rules::DomainKind::Exact => Self::Exact { name },
            meshway_rules::DomainKind::Suffix => Self::Suffix { name },
            meshway_rules::DomainKind::Subdomain => Self::Subdomain { name },
        }
    }
}

impl<A: std::fmt::Display> From<std::ops::RangeInclusive<A>> for RuleSetMember {
    fn from(range: std::ops::RangeInclusive<A>) -> Self {
        Self::IpRange {
            first: range.start().to_string(),
            last: range.end().to_string(),
        }
    }
}

impl From<&meshway_rules::Summary> for RuleSetConversion {
    fn from(summary: &meshway_rules::Summary) -> Self {
        let c = &summary.converted;
        Self {
            converted: RuleSetConverted {
                exact: count(c.exact),
                suffix: count(c.suffix),
                subdomain: count(c.subdomain),
                keyword: count(c.keyword),
                cidr: count(c.cidr),
            },
            redundant: count(summary.redundant),
            unconverted: summary
                .unconverted
                .iter()
                .map(|(reason, n)| (reason.to_string(), count(*n)))
                .collect(),
        }
    }
}

impl From<&meshway_rules::Skipped> for RuleSetSkippedEntry {
    fn from(item: &meshway_rules::Skipped) -> Self {
        Self {
            at: item.at.clone(),
            reason: item.reason.to_string(),
            entry: item.entry.clone(),
        }
    }
}

/// Counts are bounded by the 32 MiB source limit; saturate defensively.
pub fn count(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
