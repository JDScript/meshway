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
    for attempts in 1..=DUMP_ATTEMPTS {
        match attempt().await {
            Attempt::Complete(objects) => {
                if attempts > 1 {
                    tracing::debug!(operation = "dump", attempts, "Kernel dump read again");
                }
                return Ok(objects);
            }
            Attempt::Failed(error) => return Err(error),
            Attempt::Interrupted => {
                tracing::debug!(
                    operation = "dump",
                    attempts,
                    "Kernel dump interrupted; nothing read"
                );
            }
        }
    }
    tracing::debug!(
        operation = "dump",
        attempts = DUMP_ATTEMPTS,
        "Kernel dump interrupted at every attempt"
    );
    Err(Error::Netlink)
}

/// Read one attempt's answers to their end. An answer after the limit, an
/// interruption or a failure is still read: the socket's next dump starts
/// only after this one ended in the kernel.
async fn read<T>(
    mut answers: impl Stream<Item = NetlinkMessage<RouteNetlinkMessage>> + Unpin,
    take: fn(RouteNetlinkMessage) -> Option<T>,
    limit: usize,
) -> Attempt<T> {
    let mut objects = Vec::new();
    let mut interrupted = false;
    let mut failed = None;
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
            NetlinkPayload::Done(done) if done.code == 0 => {}
            NetlinkPayload::Done(done) if -done.code == EBUSY => interrupted = true,
            NetlinkPayload::Error(error) if -error.raw_code() == EBUSY => interrupted = true,
            NetlinkPayload::Done(done) => {
                tracing::debug!(
                    operation = "dump",
                    errno = -done.code,
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
        None => Attempt::Complete(objects),
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
}
