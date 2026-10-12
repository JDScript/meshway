//! Read-only views into verified rule-set content: the sets of a provider's
//! accepted catalog, the members of a bound artifact and whether it contains
//! a name or address. Matching and member order are `meshway-rules`'; query
//! names are normalized by Policy. Nothing here downloads.
use super::service::RuleSetScope;
use crate::error::Error;
use crate::secrets::CursorScope;
use meshway_policy::compile::rules::query_name;
use meshway_protocol::resources::{
    RuleSetCatalogPage, RuleSetCatalogProvider, RuleSetCatalogQuery, RuleSetCatalogSet,
    RuleSetEntriesQuery, RuleSetEntryKind, RuleSetEntryPage, RuleSetHeld, RuleSetMatchQuery,
    RuleSetMatchResponse, RuleSetMember, RuleSetUpstream, count,
};
use meshway_rules::{DomainHit, RuleSet};
use rusqlite::Transaction;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    ops::Bound,
    sync::Arc,
};

const DEFAULT_LIMIT: u16 = 100;
const MAX_CATALOG_QUERY: usize = 128;
const MAX_NAME: usize = 253;

/// Continuation of one listing (see `crate::cursor`): valid only for the
/// same query over the same content (the catalog's URL, keys, collection,
/// sequence and document digest, or the bound artifact digest).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    query: String,
    content: String,
    after: String,
}

impl Cursor {
    /// Where the listing continues and when it began (`now` for a first page).
    fn open(
        scope: &RuleSetScope,
        kind: CursorScope,
        cursor: Option<&str>,
        query: &str,
        content: &str,
        now: i64,
    ) -> Result<(Option<String>, i64), Error> {
        let Some(cursor) = cursor else {
            return Ok((None, now));
        };
        let (value, issued): (Self, i64) = crate::cursor::open(&scope.sealer, kind, cursor, now)?;
        if value.query != query || value.content != content {
            return Err(Error::InvalidCursor);
        }
        Ok((Some(value.after), issued))
    }

    fn seal(
        scope: &RuleSetScope,
        kind: CursorScope,
        query: &str,
        content: &str,
        after: String,
        issued: i64,
    ) -> Result<String, Error> {
        let value = Self {
            query: query.to_owned(),
            content: content.to_owned(),
            after,
        };
        crate::cursor::seal(&scope.sealer, kind, value, issued)
    }
}

fn user(scope: &RuleSetScope) -> Result<meshway_model::UserId, Error> {
    scope.user.ok_or(Error::PermissionDenied)
}

/// One page of the sets of a provider's accepted catalog, by name.
pub(crate) fn catalog(
    tx: &Transaction<'_>,
    scope: &RuleSetScope,
    query: &RuleSetCatalogQuery,
    now: i64,
) -> Result<RuleSetCatalogPage, Error> {
    let cursor_scope = CursorScope::RuleSetCatalog {
        user: user(scope)?,
        network: scope.network,
    };
    let q = query.q.as_deref().unwrap_or_default().to_ascii_lowercase();
    if q.chars().count() > MAX_CATALOG_QUERY || query.provider.chars().count() > 32 {
        return Err(Error::InvalidRequest);
    }
    let provider = scope
        .providers
        .get(&query.provider)
        .ok_or(Error::NotFound)?;
    let accepted = super::catalogs::accepted(tx, provider)?.ok_or(Error::CatalogNotReady)?;
    let catalog = &accepted.catalog;
    let binding = format!("{}\n{q}", query.provider);
    let content = format!(
        "{}\n{}\n{}\n{}\n{}",
        provider.catalog,
        super::rulesets::trust(&provider.public_keys),
        provider.collection,
        catalog.sequence,
        accepted.digest,
    );
    let (after, issued) = Cursor::open(
        scope,
        cursor_scope,
        query.cursor.as_deref(),
        &binding,
        &content,
        now,
    )?;
    let collection = format!("{}/", provider.collection);
    let prefix = format!("{collection}{q}");
    let matching = |start: Bound<String>| {
        catalog
            .sets
            .range((start, Bound::Unbounded))
            .take_while(|(name, _)| name.starts_with(&prefix))
    };
    let total = count(matching(Bound::Included(prefix.clone())).count() as u64);
    let start = match after {
        Some(after) => Bound::Excluded(format!("{collection}{after}")),
        None => Bound::Included(prefix.clone()),
    };
    let limit = usize::from(query.limit.unwrap_or(DEFAULT_LIMIT));
    let mut items = Vec::with_capacity(limit);
    let mut next_cursor = None;
    for (set, entry) in matching(start) {
        let name = &set[collection.len()..];
        if items.len() == limit {
            let last = items
                .last()
                .map(|item: &RuleSetCatalogSet| item.name.clone())
                .unwrap_or_default();
            next_cursor = Some(Cursor::seal(
                scope,
                cursor_scope,
                &binding,
                &content,
                last,
                issued,
            )?);
            break;
        }
        items.push(RuleSetCatalogSet {
            reference: format!("{}:{name}", query.provider),
            name: name.to_owned(),
            set: set.clone(),
            digest: entry.digest.to_string(),
            bytes: count(entry.bytes),
            entries: count(entry.entries),
            upstream: RuleSetUpstream {
                repo: entry.upstream.repo.clone(),
                reference: entry.upstream.reference.clone(),
                commit: entry.upstream.commit.clone(),
                path: entry.upstream.path.clone(),
            },
            held: entry.held.as_ref().map(|held| RuleSetHeld {
                since: held.since,
                reason: held.reason.clone(),
            }),
        });
    }
    Ok(RuleSetCatalogPage {
        provider: RuleSetCatalogProvider {
            name: query.provider.clone(),
            catalog: provider.catalog.clone(),
            collection: provider.collection.clone(),
            sequence: u32::try_from(catalog.sequence).unwrap_or(u32::MAX),
            generated_at: catalog.generated_at,
            verified_at: accepted.verified_at,
        },
        items,
        total,
        next_cursor,
    })
}

/// The content currently bound to `key`; 404 for an unknown key or one
/// without content, as the report.
fn bound(
    tx: &Transaction<'_>,
    scope: &RuleSetScope,
    key: &str,
) -> Result<(meshway_rules::Digest, Arc<RuleSet>), Error> {
    if !scope.definitions.contains_key(key) {
        return Err(Error::NotFound);
    }
    let digest: meshway_rules::Digest = scope
        .views
        .get(key)
        .and_then(|view| view.artifact.as_ref())
        .ok_or(Error::NotFound)?
        .digest
        .parse()
        .map_err(|_| Error::Internal("bound digest"))?;
    Ok((digest, super::artifacts::load(tx, &digest)?))
}

/// One page of one kind of members of `key`'s bound content.
pub(crate) fn entries(
    tx: &Transaction<'_>,
    scope: &RuleSetScope,
    key: &str,
    query: &RuleSetEntriesQuery,
    now: i64,
) -> Result<RuleSetEntryPage, Error> {
    let cursor_scope = CursorScope::RuleSetEntries {
        user: user(scope)?,
        network: scope.network,
    };
    let (digest, set) = bound(tx, scope, key)?;
    let q = query.q.as_deref().unwrap_or_default();
    if q.chars().count() > MAX_NAME {
        return Err(Error::InvalidRequest);
    }
    let q = match query.kind {
        RuleSetEntryKind::Domain => query_name(q),
        RuleSetEntryKind::Keyword => q.to_ascii_lowercase(),
        RuleSetEntryKind::Ipv4 | RuleSetEntryKind::Ipv6 => q.to_owned(),
    };
    let kind = match query.kind {
        RuleSetEntryKind::Domain => "domain",
        RuleSetEntryKind::Keyword => "keyword",
        RuleSetEntryKind::Ipv4 => "ipv4",
        RuleSetEntryKind::Ipv6 => "ipv6",
    };
    let binding = format!("{key}\n{kind}\n{q}");
    let content = digest.to_string();
    let (after, issued) = Cursor::open(
        scope,
        cursor_scope,
        query.cursor.as_deref(),
        &binding,
        &content,
        now,
    )?;
    let limit = usize::from(query.limit.unwrap_or(DEFAULT_LIMIT));
    // Each listing yields its members with the position to resume after
    // them; one more than a page tells whether another page exists.
    let index = |after: &Option<String>| -> Result<usize, Error> {
        after
            .as_deref()
            .map_or(Ok(0), |a| a.parse().map_err(|_| Error::InvalidCursor))
    };
    let mut page: Vec<(RuleSetMember, String)> = match query.kind {
        RuleSetEntryKind::Domain => set
            .domains_ending_with(&q, after.as_deref())
            .take(limit + 1)
            .map(|entry| {
                let position = entry.name.clone();
                (entry.into(), position)
            })
            .collect(),
        RuleSetEntryKind::Keyword => set
            .keywords()
            .enumerate()
            .skip(index(&after)?)
            .filter(|(_, keyword)| keyword.starts_with(&q))
            .take(limit + 1)
            .map(|(i, keyword)| {
                (
                    RuleSetMember::Keyword {
                        keyword: keyword.to_owned(),
                    },
                    (i + 1).to_string(),
                )
            })
            .collect(),
        RuleSetEntryKind::Ipv4 => {
            let start = match after {
                Some(_) => index(&after)?,
                None if q.is_empty() => 0,
                None => {
                    let at: Ipv4Addr = q.parse().map_err(|_| Error::InvalidRequest)?;
                    set.ipv4_ranges()
                        .position(|range| *range.end() >= at)
                        .unwrap_or(usize::MAX)
                }
            };
            ranges(set.ipv4_ranges(), start, limit)
        }
        RuleSetEntryKind::Ipv6 => {
            let start = match after {
                Some(_) => index(&after)?,
                None if q.is_empty() => 0,
                None => {
                    let at: Ipv6Addr = q.parse().map_err(|_| Error::InvalidRequest)?;
                    set.ipv6_ranges()
                        .position(|range| *range.end() >= at)
                        .unwrap_or(usize::MAX)
                }
            };
            ranges(set.ipv6_ranges(), start, limit)
        }
    };
    let next_cursor = if page.len() > limit {
        page.truncate(limit);
        let after = page.last().map(|(_, at)| at.clone()).unwrap_or_default();
        Some(Cursor::seal(
            scope,
            cursor_scope,
            &binding,
            &content,
            after,
            issued,
        )?)
    } else {
        None
    };
    Ok(RuleSetEntryPage {
        key: key.to_owned(),
        digest: content,
        kind: query.kind,
        counts: set.counts().into(),
        items: page.into_iter().map(|(member, _)| member).collect(),
        next_cursor,
    })
}

fn ranges<A: std::fmt::Display>(
    ranges: impl Iterator<Item = std::ops::RangeInclusive<A>>,
    start: usize,
    limit: usize,
) -> Vec<(RuleSetMember, String)> {
    ranges
        .enumerate()
        .skip(start)
        .take(limit + 1)
        .map(|(i, range)| (range.into(), (i + 1).to_string()))
        .collect()
}

/// Whether `key`'s bound content contains the queried name or address, and
/// the member that decides it.
pub(crate) fn matches(
    tx: &Transaction<'_>,
    scope: &RuleSetScope,
    key: &str,
    query: &RuleSetMatchQuery,
) -> Result<RuleSetMatchResponse, Error> {
    let (digest, set) = bound(tx, scope, key)?;
    let (query, member) = match (&query.name, &query.ip) {
        (Some(name), None) => {
            if name.chars().count() > MAX_NAME + 1 {
                return Err(Error::InvalidRequest);
            }
            let name = query_name(name);
            if name.is_empty() || name.len() > MAX_NAME {
                return Err(Error::InvalidRequest);
            }
            let tail = |len: usize| {
                name.get(name.len().saturating_sub(len)..)
                    .map(str::to_owned)
                    .ok_or(Error::Internal("matched suffix"))
            };
            let member = match set.match_domain(&name) {
                None => None,
                Some(DomainHit::Exact) => Some(RuleSetMember::Exact { name: name.clone() }),
                Some(DomainHit::Suffix { len }) => Some(RuleSetMember::Suffix { name: tail(len)? }),
                Some(DomainHit::Subdomain { len }) => {
                    Some(RuleSetMember::Subdomain { name: tail(len)? })
                }
                Some(DomainHit::Keyword { index }) => Some(RuleSetMember::Keyword {
                    keyword: set
                        .keyword(index)
                        .ok_or(Error::Internal("matched keyword"))?
                        .to_owned(),
                }),
            };
            (name, member)
        }
        (None, Some(ip)) => {
            if ip.chars().count() > 64 {
                return Err(Error::InvalidRequest);
            }
            let address: IpAddr = ip.parse().map_err(|_| Error::InvalidRequest)?;
            (
                address.to_string(),
                set.ip_range(address).map(RuleSetMember::from),
            )
        }
        _ => return Err(Error::InvalidRequest),
    };
    Ok(RuleSetMatchResponse {
        key: key.to_owned(),
        digest: digest.to_string(),
        query,
        member,
    })
}
