//! The one rtnetlink read layer: request connections and their dumps.
//!
//! The kernel runs one dump per netlink socket at a time; a dump started on
//! a socket while another one is running there is refused with `EBUSY`
//! (`netlink_dump_start`). A dump during which the dumped table changed is
//! marked `NLM_F_DUMP_INTR`, on any of its messages or only on its
//! `NLMSG_DONE`. Neither says anything about the table: both are a read that
//! did not finish. The owner tasks of one term (Host, capture owner, exit
//! owner) share a request socket, so its dumps are taken one at a time here,
//! and an interrupted dump is read again, at most [`DUMP_ATTEMPTS`] times in
//! all, before the read fails as unavailable like any other failed read.
use super::{Error, Result, netlink_failure, netlink_timeout};
use futures_util::{Stream, StreamExt};
use rtnetlink::{
    IpVersion,
    packet_core::{NLM_F_DUMP, NLM_F_DUMP_INTR, NLM_F_REQUEST, NetlinkMessage, NetlinkPayload},
    packet_route::{
        AddressFamily, RouteNetlinkMessage,
        address::AddressMessage,
        link::LinkMessage,
        route::{RouteFlags, RouteMessage},
        rule::RuleMessage,
    },
    proto::Connection,
};
use std::{ops::Deref, sync::Arc, time::Duration};

/// One dump and its rereads after an interruption.
pub(in crate::linux::kernel) const DUMP_ATTEMPTS: u32 = 4;
/// All attempts of one dump, waiting for the socket included.
const DUMP_BUDGET: Duration = Duration::from_secs(5);
const EBUSY: i32 = 16;

/// A request socket of this layer: requests as [`rtnetlink::Handle`], dumps
/// only through [`dump`], one at a time per socket.
#[derive(Clone)]
pub(in crate::linux) struct Handle {
    inner: rtnetlink::Handle,
    dumps: Arc<tokio::sync::Mutex<()>>,
}

impl Deref for Handle {
    type Target = rtnetlink::Handle;
    fn deref(&self) -> &rtnetlink::Handle {
        &self.inner
    }
}

/// A new request socket without multicast subscriptions, and its
/// connection task. Its dumps end with their `NLMSG_DONE`, which carries the
/// last interruption mark; a dump of this socket read past [`dump`] would
/// see it as an unexpected message.
pub(in crate::linux) fn connect() -> std::io::Result<(Connection<RouteNetlinkMessage>, Handle)> {
    let (mut connection, inner, unsolicited) = rtnetlink::new_connection()?;
    drop(unsolicited);
    connection.set_forward_done(true);
    Ok((
        connection,
        Handle {
            inner,
            dumps: Arc::new(tokio::sync::Mutex::new(())),
        },
    ))
}

/// How one attempt of a dump ended.
#[derive(Debug)]
enum Attempt<T> {
    Complete(Vec<T>),
    /// `EBUSY` or `NLM_F_DUMP_INTR`: read again.
    Interrupted,
    Failed(Error),
}

/// Dump `request` (built anew for each attempt), keeping what `take`
/// selects, at most `limit` objects.
async fn dump<T>(
    handle: &Handle,
    request: impl Fn() -> RouteNetlinkMessage,
    take: fn(RouteNetlinkMessage) -> Option<T>,
    limit: usize,
) -> Result<Vec<T>> {
    tokio::time::timeout(DUMP_BUDGET, async {
        let _socket = handle.dumps.lock().await;
        let request = &request;
        reread(move || async move {
            #[cfg(test)]
            if super::tool::command::faults::take(&super::tool::command::faults::INTERRUPTED_DUMPS)
            {
                // As if the kernel answered `EBUSY`: nothing was read.
                return Attempt::Interrupted;
            }
            let mut message = NetlinkMessage::from(request());
            message.header.flags = NLM_F_REQUEST | NLM_F_DUMP;
            match handle.inner.clone().request(message) {
                Ok(answers) => read(answers, take, limit).await,
                Err(_) => Attempt::Failed(Error::Netlink),
            }
        })
        .await
    })
    .await
    .map_err(|_| netlink_timeout("dump"))?
}

/// Run `attempt` until it completes, at most [`DUMP_ATTEMPTS`] times while it
/// is interrupted. Every attempt is read to its end before the next starts.
async fn reread<T, F, A>(mut attempt: F) -> Result<Vec<T>>
where
    F: FnMut() -> A,
    A: Future<Output = Attempt<T>>,
{
    unfinished(
        "dump",
        || {
            let attempt = attempt();
            async move {
                match attempt.await {
                    Attempt::Complete(objects) => Some(Ok(objects)),
                    Attempt::Failed(error) => Some(Err(error)),
                    Attempt::Interrupted => None,
                }
            }
        },
        Error::Netlink,
    )
    .await
}

/// The one rule for a kernel read that did not finish (any netlink family):
/// `attempt` answers `None` when it was interrupted, which proves nothing,
/// and it runs again, at most [`DUMP_ATTEMPTS`] times in all; then the read
/// fails with `exhausted`, unavailable like any other failed read.
pub(in crate::linux::kernel) async fn unfinished<T, E, F, A>(
    operation: &'static str,
    mut attempt: F,
    exhausted: E,
) -> std::result::Result<T, E>
where
    F: FnMut() -> A,
    A: Future<Output = Option<std::result::Result<T, E>>>,
{
    for attempts in 1..=DUMP_ATTEMPTS {
        match attempt().await {
            Some(Ok(read)) => {
                if attempts > 1 {
                    tracing::debug!(operation, attempts, "Kernel dump read again");
                }
                return Ok(read);
            }
            Some(Err(error)) => return Err(error),
            None => {
                tracing::debug!(operation, attempts, "Kernel dump interrupted; nothing read");
            }
        }
    }
    tracing::debug!(
        operation,
        attempts = DUMP_ATTEMPTS,
        "Kernel dump interrupted at every attempt"
    );
    Err(exhausted)
}

/// Read one attempt's answers to their end. An answer after the limit, an
/// interruption or a failure is still read: the socket's next dump starts
/// only after this one ended in the kernel. Only a successful `NLMSG_DONE`
/// completes the dump: answers that end without one (the socket closed or
/// its stream ended) are a read that failed, never the whole table.
async fn read<T>(
    mut answers: impl Stream<Item = NetlinkMessage<RouteNetlinkMessage>> + Unpin,
    take: fn(RouteNetlinkMessage) -> Option<T>,
    limit: usize,
) -> Attempt<T> {
    let mut objects = Vec::new();
    let mut interrupted = false;
    let mut failed = None;
    let mut done = false;
    while let Some(answer) = answers.next().await {
        interrupted |= answer.header.flags & NLM_F_DUMP_INTR != 0;
        match answer.payload {
            NetlinkPayload::InnerMessage(message) => match take(message) {
                Some(_) if objects.len() == limit => {
                    failed.get_or_insert(Error::Capacity);
                }
                Some(object) => objects.push(object),
                None => {
                    failed.get_or_insert(Error::Netlink);
                }
            },
            NetlinkPayload::Done(end) if end.code == 0 => done = true,
            NetlinkPayload::Done(end) if -end.code == EBUSY => interrupted = true,
            NetlinkPayload::Error(error) if -error.raw_code() == EBUSY => interrupted = true,
            NetlinkPayload::Done(end) => {
                tracing::debug!(
                    operation = "dump",
                    errno = -end.code,
                    "Kernel netlink operation failed"
                );
                failed.get_or_insert(Error::Netlink);
            }
            NetlinkPayload::Error(error) => {
                failed.get_or_insert(netlink_failure(
                    "dump",
                    rtnetlink::Error::NetlinkError(error),
                ));
            }
            _ => {
                failed.get_or_insert(Error::Netlink);
            }
        }
    }
    match failed {
        Some(error) => Attempt::Failed(error),
        None if interrupted => Attempt::Interrupted,
        None if done => Attempt::Complete(objects),
        None => {
            tracing::debug!(
                operation = "dump",
                objects = objects.len(),
                "Kernel dump ended without its end message"
            );
            Attempt::Failed(Error::Netlink)
        }
    }
}

pub(super) async fn rules(
    handle: &Handle,
    version: IpVersion,
    limit: usize,
) -> Result<Vec<RuleMessage>> {
    let family = match version {
        IpVersion::V4 => AddressFamily::Inet,
        IpVersion::V6 => AddressFamily::Inet6,
    };
    dump(
        handle,
        || {
            let mut message = RuleMessage::default();
            message.header.family = family;
            RouteNetlinkMessage::GetRule(message)
        },
        |message| match message {
            RouteNetlinkMessage::NewRule(rule) => Some(rule),
            _ => None,
        },
        limit,
    )
    .await
}

/// Route objects of `family` in every table, cached exceptions included.
pub(super) async fn routes(
    handle: &Handle,
    family: AddressFamily,
    limit: usize,
) -> Result<Vec<RouteMessage>> {
    dump(
        handle,
        || {
            let mut message = RouteMessage::default();
            message.header.address_family = family;
            RouteNetlinkMessage::GetRoute(message)
        },
        |message| match message {
            RouteNetlinkMessage::NewRoute(route) => Some(route),
            _ => None,
        },
        limit,
    )
    .await
    .map(|routes| {
        routes
            .into_iter()
            .filter(|route| !route.header.flags.contains(RouteFlags::Cloned))
            .collect()
    })
}

pub(super) async fn addresses(handle: &Handle, limit: usize) -> Result<Vec<AddressMessage>> {
    dump(
        handle,
        || RouteNetlinkMessage::GetAddress(AddressMessage::default()),
        |message| match message {
            RouteNetlinkMessage::NewAddress(address) => Some(address),
            _ => None,
        },
        limit,
    )
    .await
}

pub(super) async fn links(handle: &Handle, limit: usize) -> Result<Vec<LinkMessage>> {
    dump(
        handle,
        || RouteNetlinkMessage::GetLink(LinkMessage::default()),
        |message| match message {
            RouteNetlinkMessage::NewLink(link) => Some(link),
            _ => None,
        },
        limit,
    )
    .await
}

/// The links of this namespace whose index, name or alias is the given one,
/// fully decoded; every other link of the dump is only scanned for those
/// three fields (G16). A periodic proof then costs what it compares, not the
/// configuration of unrelated interfaces: the route codec renders each
/// attribute of a bridge or TUN while decoding it. The dump has its own
/// socket (no other dump can be running on it) and the same rules as
/// [`dump`]: an interrupted one is read again, at most [`DUMP_ATTEMPTS`]
/// times, within [`DUMP_BUDGET`]; more than `limit` links is `Capacity`.
pub(super) async fn links_named(
    index: u32,
    name: &str,
    alias: &str,
    limit: usize,
) -> Result<Vec<LinkMessage>> {
    tokio::time::timeout(
        DUMP_BUDGET,
        unfinished(
            "dump",
            || async {
                #[cfg(test)]
                if super::tool::command::faults::take(
                    &super::tool::command::faults::INTERRUPTED_DUMPS,
                ) {
                    return None;
                }
                match named_once(index, name, alias, limit).await {
                    Ok(Some(links)) => Some(Ok(links)),
                    Ok(None) => None,
                    Err(error) => Some(Err(error)),
                }
            },
            Error::Netlink,
        ),
    )
    .await
    .map_err(|_| netlink_timeout("dump"))?
}

const NLMSG_HEADER: usize = 16;
const IFINFOMSG: usize = 16;
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const RTM_NEWLINK: u16 = 16;
const RTM_GETLINK: u16 = 18;
const IFLA_IFNAME: u16 = 3;
const IFLA_IFALIAS: u16 = 20;

/// One attempt of [`links_named`]: `None` when it was interrupted.
async fn named_once(
    index: u32,
    name: &str,
    alias: &str,
    limit: usize,
) -> Result<Option<Vec<LinkMessage>>> {
    use rtnetlink::sys::{Socket, SocketAddr, protocols::NETLINK_ROUTE};
    let mut socket = Socket::new(NETLINK_ROUTE)?;
    socket.set_non_blocking(true)?;
    socket.bind_auto()?;
    // RUSTSEC-2026-0386: the AsyncFd owns this fd for life; nothing dups or closes it.
    #[allow(deprecated)]
    let socket = tokio::io::unix::AsyncFd::new(socket)?;
    let sequence = 1;
    let mut request = vec![0; NLMSG_HEADER + IFINFOMSG];
    let length = u32::try_from(request.len()).map_err(|_| Error::Netlink)?;
    request[0..4].copy_from_slice(&length.to_ne_bytes());
    request[4..6].copy_from_slice(&RTM_GETLINK.to_ne_bytes());
    request[6..8].copy_from_slice(&(NLM_F_REQUEST | NLM_F_DUMP).to_ne_bytes());
    request[8..12].copy_from_slice(&u32::to_ne_bytes(sequence));
    if socket
        .get_ref()
        .send_to(&request, &SocketAddr::new(0, 0), 0)?
        != request.len()
    {
        return Err(Error::Netlink);
    }
    let mut buffer = vec![0; 65_536];
    let mut scan = Named::new(index, name, alias, limit);
    loop {
        let mut ready = socket.readable().await?;
        let (size, source) = match socket.get_ref().recv_from(&mut &mut buffer[..], 0) {
            Ok(received) => received,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                ready.clear_ready();
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if source.port_number() != 0 {
            continue;
        }
        if size == 0 || size >= buffer.len() {
            return Err(Error::Netlink);
        }
        if let Some(done) = scan.datagram(&buffer[..size], sequence)? {
            return Ok(done);
        }
    }
}

/// The scan of one link dump: header, `IFLA_IFNAME` and `IFLA_IFALIAS` of
/// every link, the full decode of matching ones only.
struct Named<'a> {
    index: u32,
    name: &'a [u8],
    alias: &'a [u8],
    limit: usize,
    seen: usize,
    interrupted: bool,
    failed: Option<Error>,
    links: Vec<LinkMessage>,
    #[cfg(test)]
    decoded: usize,
}

impl<'a> Named<'a> {
    fn new(index: u32, name: &'a str, alias: &'a str, limit: usize) -> Self {
        Self {
            index,
            name: name.as_bytes(),
            alias: alias.as_bytes(),
            limit,
            seen: 0,
            interrupted: false,
            failed: None,
            links: Vec::new(),
            #[cfg(test)]
            decoded: 0,
        }
    }

    /// One datagram; `Some` once the dump ended (`None` inside: it was
    /// interrupted). Every message is read to the end of the dump.
    fn datagram(
        &mut self,
        mut datagram: &[u8],
        sequence: u32,
    ) -> Result<Option<Option<Vec<LinkMessage>>>> {
        while datagram.len() >= NLMSG_HEADER {
            let word = |at: usize| -> Result<[u8; 4]> {
                datagram
                    .get(at..at + 4)
                    .and_then(|bytes| bytes.try_into().ok())
                    .ok_or(Error::Netlink)
            };
            let length =
                usize::try_from(u32::from_ne_bytes(word(0)?)).map_err(|_| Error::Netlink)?;
            if length < NLMSG_HEADER || length > datagram.len() {
                return Err(Error::Netlink);
            }
            let kind = u16::from_ne_bytes([datagram[4], datagram[5]]);
            let flags = u16::from_ne_bytes([datagram[6], datagram[7]]);
            if u32::from_ne_bytes(word(8)?) != sequence {
                return Err(Error::Netlink);
            }
            self.interrupted |= flags & NLM_F_DUMP_INTR != 0;
            let message = &datagram[..length];
            match kind {
                NLMSG_DONE | NLMSG_ERROR => {
                    let code = message
                        .get(NLMSG_HEADER..NLMSG_HEADER + 4)
                        .and_then(|bytes| bytes.try_into().ok())
                        .map_or(0, i32::from_ne_bytes);
                    if -code == EBUSY {
                        self.interrupted = true;
                    } else if code != 0 || kind == NLMSG_ERROR {
                        self.failed.get_or_insert(Error::Netlink);
                    }
                    if let Some(error) = self.failed.take() {
                        return Err(error);
                    }
                    return Ok(Some(
                        (!self.interrupted).then(|| std::mem::take(&mut self.links)),
                    ));
                }
                RTM_NEWLINK => self.link(message),
                _ => {
                    self.failed.get_or_insert(Error::Netlink);
                }
            }
            datagram = datagram.get((length + 3) & !3..).unwrap_or_default();
        }
        Ok(None)
    }

    fn link(&mut self, message: &[u8]) {
        self.seen += 1;
        if self.seen > self.limit {
            self.failed.get_or_insert(Error::Capacity);
            return;
        }
        let Some(header) = message.get(NLMSG_HEADER..NLMSG_HEADER + IFINFOMSG) else {
            self.failed.get_or_insert(Error::Netlink);
            return;
        };
        let index = u32::from_ne_bytes([header[4], header[5], header[6], header[7]]);
        let mut named = index == self.index;
        for attribute in
            rtnetlink::packet_core::NlasIterator::new(&message[NLMSG_HEADER + IFINFOMSG..])
        {
            let Ok(attribute) = attribute else {
                self.failed.get_or_insert(Error::Netlink);
                return;
            };
            let value = attribute.value();
            let value = value.strip_suffix(&[0]).unwrap_or(value);
            named |= (attribute.kind() == IFLA_IFNAME && value == self.name)
                || (attribute.kind() == IFLA_IFALIAS && value == self.alias);
        }
        if !named {
            return;
        }
        #[cfg(test)]
        {
            self.decoded += 1;
        }
        match NetlinkMessage::<RouteNetlinkMessage>::deserialize(message) {
            Ok(NetlinkMessage {
                payload: NetlinkPayload::InnerMessage(RouteNetlinkMessage::NewLink(link)),
                ..
            }) => self.links.push(link),
            _ => {
                self.failed.get_or_insert(Error::Netlink);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtnetlink::packet_core::{DoneMessage, ErrorMessage, NLM_F_MULTIPART};
    use std::num::NonZeroI32;

    fn link(index: u32, flags: u16) -> NetlinkMessage<RouteNetlinkMessage> {
        let mut link = LinkMessage::default();
        link.header.index = index;
        let mut message = NetlinkMessage::from(RouteNetlinkMessage::NewLink(link));
        message.header.flags = NLM_F_MULTIPART | flags;
        message
    }

    fn done(code: i32, flags: u16) -> NetlinkMessage<RouteNetlinkMessage> {
        let mut done = DoneMessage::default();
        done.code = code;
        let mut message = NetlinkMessage::new(Default::default(), NetlinkPayload::Done(done));
        message.header.flags = NLM_F_MULTIPART | flags;
        message
    }

    fn refused(errno: i32) -> NetlinkMessage<RouteNetlinkMessage> {
        let mut error = ErrorMessage::default();
        error.code = NonZeroI32::new(-errno);
        NetlinkMessage::new(Default::default(), NetlinkPayload::Error(error))
    }

    fn take(message: RouteNetlinkMessage) -> Option<u32> {
        match message {
            RouteNetlinkMessage::NewLink(link) => Some(link.header.index),
            _ => None,
        }
    }

    async fn attempt(
        answers: Vec<NetlinkMessage<RouteNetlinkMessage>>,
        limit: usize,
    ) -> Attempt<u32> {
        read(futures_util::stream::iter(answers), take, limit).await
    }

    #[tokio::test]
    async fn busy_and_interrupted_dumps_are_unfinished_reads_and_failures_are_not() {
        assert!(matches!(
            attempt(vec![link(1, 0), link(2, 0), done(0, 0)], 8).await,
            Attempt::Complete(links) if links == [1, 2]
        ));
        // Another dump runs on the socket.
        assert!(matches!(
            attempt(vec![refused(EBUSY)], 8).await,
            Attempt::Interrupted
        ));
        // The table changed during the dump: marked on a message, or on the
        // end alone.
        assert!(matches!(
            attempt(
                vec![
                    link(1, 0),
                    link(2, NLM_F_DUMP_INTR),
                    done(0, NLM_F_DUMP_INTR)
                ],
                8
            )
            .await,
            Attempt::Interrupted
        ));
        assert!(matches!(
            attempt(vec![link(1, 0), done(0, NLM_F_DUMP_INTR)], 8).await,
            Attempt::Interrupted
        ));
        assert!(matches!(
            attempt(vec![link(1, 0), done(-EBUSY, 0)], 8).await,
            Attempt::Interrupted
        ));
        // A refusal, a failed end and a full dump are failures, read to
        // the end, never reread.
        assert!(matches!(
            attempt(vec![refused(1)], 8).await,
            Attempt::Failed(Error::Netlink)
        ));
        assert!(matches!(
            attempt(vec![link(1, 0), done(-105, 0)], 8).await,
            Attempt::Failed(Error::Netlink)
        ));
        assert!(matches!(
            attempt(vec![link(1, 0), link(2, NLM_F_DUMP_INTR), done(0, 0)], 1).await,
            Attempt::Failed(Error::Capacity)
        ));
    }

    /// F8a F8: only a successful end completes a dump. Answers that stop
    /// without one, after some objects or at once, are a failed read
    /// (unavailable), never the whole table.
    #[tokio::test]
    async fn a_dump_without_its_end_is_a_failed_read_not_a_complete_one() {
        assert!(matches!(
            attempt(vec![link(1, 0), link(2, 0)], 8).await,
            Attempt::Failed(Error::Netlink)
        ));
        assert!(matches!(
            attempt(Vec::new(), 8).await,
            Attempt::Failed(Error::Netlink)
        ));
        assert!(matches!(
            attempt(vec![done(0, 0)], 8).await,
            Attempt::Complete(links) if links.is_empty()
        ));
    }

    #[tokio::test]
    async fn an_interrupted_dump_is_read_again_within_its_bound() {
        let mut answers = vec![
            Attempt::Interrupted,
            Attempt::Interrupted,
            Attempt::Complete(vec![7]),
        ]
        .into_iter();
        let mut attempts = 0;
        let read = reread(|| {
            attempts += 1;
            std::future::ready(answers.next().unwrap())
        })
        .await;
        assert_eq!(read.unwrap(), [7]);
        assert_eq!(attempts, 3);
        // Interrupted at every attempt: unavailable, after the bound.
        let mut attempts = 0;
        let read = reread(|| {
            attempts += 1;
            std::future::ready(Attempt::<u32>::Interrupted)
        })
        .await;
        assert!(matches!(read, Err(Error::Netlink)));
        assert_eq!(attempts, DUMP_ATTEMPTS);
        // A failure is not read again.
        let mut attempts = 0;
        let read = reread(|| {
            attempts += 1;
            std::future::ready(Attempt::<u32>::Failed(Error::Conflict))
        })
        .await;
        assert!(matches!(read, Err(Error::Conflict)));
        assert_eq!(attempts, 1);
    }

    /// The kernel's own refusal, in an isolated namespace: dumps of one
    /// socket started while another one runs there are refused `EBUSY`
    /// (rtnetlink's own requests, all sent at once); through this layer the
    /// same dumps of one socket all complete.
    #[tokio::test]
    #[ignore = "requires an isolated Linux network namespace and NET_ADMIN"]
    async fn dumps_of_one_socket_are_taken_one_at_a_time() {
        use futures_util::TryStreamExt;
        use rtnetlink::packet_route::rule::{RuleAction, RuleAttribute};
        assert_eq!(
            std::env::var("MESHWAY_UNDERLAY_ISOLATED").as_deref(),
            Ok("1")
        );
        const RULES: u32 = 4000;
        const DUMPS: usize = 16;
        let (connection, handle) = connect().unwrap();
        let task = tokio::spawn(connection);
        // Enough rules that one dump spans many reads of the socket.
        let rule = |index: u32| {
            let mut rule = RuleMessage::default();
            rule.header.family = AddressFamily::Inet;
            rule.header.action = RuleAction::ToTable;
            rule.header.src_len = 32;
            rule.attributes = vec![
                RuleAttribute::Priority(31_000),
                RuleAttribute::Table(31_000),
                RuleAttribute::Source(std::net::Ipv4Addr::from(0xc612_0000 + index).into()),
            ];
            rule
        };
        for index in 0..RULES {
            let mut request = handle.rule().add();
            *request.message_mut() = rule(index);
            request.execute().await.unwrap();
        }
        // rtnetlink's own dumps, on a socket of its own.
        let (plain, unlayered, unsolicited) = rtnetlink::new_connection().unwrap();
        drop(unsolicited);
        let plain = tokio::spawn(plain);
        let raw = futures_util::future::join_all((0..DUMPS).map(|_| {
            unlayered
                .rule()
                .get(IpVersion::V4)
                .execute()
                .try_collect::<Vec<_>>()
        }))
        .await;
        let busy = raw
            .iter()
            .filter(|dump| {
                matches!(dump, Err(rtnetlink::Error::NetlinkError(error)) if -error.raw_code() == EBUSY)
            })
            .count();
        let layered = futures_util::future::join_all(
            (0..DUMPS).map(|_| rules(&handle, IpVersion::V4, 65_536)),
        )
        .await;
        let complete = layered
            .iter()
            .filter(|dump| {
                dump.as_ref()
                    .is_ok_and(|rules| rules.len() >= RULES as usize)
            })
            .count();
        for index in 0..RULES {
            handle.rule().del(rule(index)).execute().await.unwrap();
        }
        println!(
            "X16_SOCKET_DUMPS=raw-busy-{busy}-of-{DUMPS},layered-complete-{complete}-of-{DUMPS}"
        );
        assert!(busy > 0, "the kernel refused no concurrent dump");
        assert_eq!(complete, DUMPS);
        drop((handle, unlayered));
        task.abort();
        plain.abort();
    }

    /// RC-1 G16 (P2: one link dump decoded 528 µs on it7, 0.6 µs reading
    /// only index, name and alias): the anchor proof fully decodes only the
    /// links with its index, name or alias, however many unrelated (and
    /// however richly configured) links the namespace has; an interrupted
    /// dump is no answer, and the limit counts every link.
    #[test]
    fn a_named_link_scan_decodes_only_the_links_it_compares() {
        use rtnetlink::packet_route::link::{InfoKind, LinkAttribute, LinkInfo};
        let bytes = |mut message: NetlinkMessage<RouteNetlinkMessage>| {
            message.header.sequence_number = 1;
            message.finalize();
            let mut buffer = vec![0; message.buffer_len()];
            message.serialize(&mut buffer);
            buffer
        };
        let named = |index: u32, name: &str, alias: Option<&str>| {
            let mut link = LinkMessage::default();
            link.header.index = index;
            link.attributes.push(LinkAttribute::IfName(name.into()));
            if let Some(alias) = alias {
                link.attributes.push(LinkAttribute::IfAlias(alias.into()));
            }
            link.attributes.push(LinkAttribute::Mtu(1500));
            link.attributes
                .push(LinkAttribute::LinkInfo(vec![LinkInfo::Kind(
                    InfoKind::Bridge,
                )]));
            bytes(NetlinkMessage::from(RouteNetlinkMessage::NewLink(link)))
        };
        for unrelated in [2, 200] {
            let mut datagram = Vec::new();
            for index in 0..unrelated {
                datagram.extend(named(100 + index, &format!("br{index}"), Some("other")));
            }
            datagram.extend(named(7, "mwdd1", Some("meshway-dns")));
            datagram.extend(named(300, "renamed", Some("meshway-dns")));
            datagram.extend(bytes(done(0, 0)));
            let mut scan = Named::new(7, "mwdd1", "meshway-dns", 65_536);
            let links = scan.datagram(&datagram, 1).unwrap().unwrap().unwrap();
            assert_eq!(
                links
                    .iter()
                    .map(|link| link.header.index)
                    .collect::<Vec<_>>(),
                [7, 300]
            );
            assert_eq!(
                scan.decoded, 2,
                "{unrelated} unrelated links decoded nothing"
            );
            let mut small = Named::new(7, "mwdd1", "meshway-dns", 2);
            assert!(matches!(small.datagram(&datagram, 1), Err(Error::Capacity)));
        }
        let mut datagram = named(7, "mwdd1", None);
        datagram.extend(bytes(done(0, NLM_F_DUMP_INTR)));
        let mut scan = Named::new(7, "mwdd1", "meshway-dns", 16);
        assert!(
            matches!(scan.datagram(&datagram, 1), Ok(Some(None))),
            "an interrupted dump is read again"
        );
    }
}
