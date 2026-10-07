//! One protected, typed watchNode connection. The host owns its task and retry.
use crate::{Control, Error};
use meshway_control_client::{Client, ClientError, EventStream, StreamOptions};
use meshway_protocol::{ErrorResponse, NodeEvent, StreamEvent, WireSecret, events::ClosedReason};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// A notice retains its authentication scope even while queued at the host.
pub struct EventNotice {
    owner: Arc<()>,
    scope: CancellationToken,
    result: Result<NodeEvent, ClientError<ErrorResponse>>,
}

/// Single connection; `next` has no retry loop. Drop cancels its protected HTTP
/// request. No cursor is retained because event projections are not our cache.
pub struct NodeEvents {
    owner: Arc<()>,
    client: Client,
    token: WireSecret,
    scope: CancellationToken,
    stream: Option<EventStream<NodeEvent, ErrorResponse>>,
    ended: bool,
}

impl NodeEvents {
    pub async fn next(&mut self) -> Option<EventNotice> {
        if self.ended || self.scope.is_cancelled() {
            self.stream = None;
            return None;
        }
        let result = tokio::select! { biased;
            _ = self.scope.cancelled() => None,
            result = async {
                if self.stream.is_none() {
                    match self.client.watch_node(self.token.expose(), &StreamOptions::default()).await {
                        Ok(stream) => self.stream = Some(stream),
                        Err(error) => return Some(Err(error)),
                    }
                }
                self.stream.as_mut().expect("opened stream").next().await.map(|result| result.map(|delivery| delivery.data))
            } => result,
        };
        self.ended = result.as_ref().is_none_or(|result| match result {
            Ok(event) => matches!(event.0, StreamEvent::Closed { .. }),
            Err(_) => true,
        });
        if self.ended {
            self.stream = None;
        }
        result.map(|result| EventNotice {
            owner: self.owner.clone(),
            scope: self.scope.clone(),
            result,
        })
    }
}

impl Control {
    pub(super) fn reset_events(&mut self) {
        self.events_cancel.cancel();
        self.events_cancel = CancellationToken::new();
    }

    /// Uses precisely the same generated client and host-protected socket pool
    /// as every other control operation. The host may move this into one task.
    pub fn watch_node(&self) -> Result<NodeEvents, Error> {
        self.current_instance()?;
        Ok(NodeEvents {
            owner: self.application_owner.clone(),
            client: self.client.clone(),
            token: WireSecret::new(self.token()?.to_owned()),
            scope: self.events_cancel.clone(),
            stream: None,
            ended: false,
        })
    }

    /// True requests the existing refresh cycle. Events never install a target,
    /// acknowledge a cursor, mint authority, or extend any deadline.
    pub fn receive_event(&mut self, notice: EventNotice) -> Result<bool, Error> {
        if !Arc::ptr_eq(&notice.owner, &self.application_owner) || notice.scope.is_cancelled() {
            return Ok(false);
        }
        let event = match notice.result {
            Ok(event) => event,
            Err(error) => {
                // Only an actual HTTP authentication rejection fences authority.
                // EOF, malformed events and delivery failure are watch failures,
                // not evidence that an independently granted lease was revoked.
                if matches!(
                    &error,
                    ClientError::Http {
                        status: 401 | 403 | 404 | 409,
                        ..
                    } | ClientError::Network {
                        status: Some(401 | 403 | 404 | 409)
                    }
                ) {
                    return self.registered::<bool>(Err(error));
                }
                return Ok(false);
            }
        };
        match event.0 {
            StreamEvent::Closed { reason } => match reason {
                ClosedReason::SessionExpired | ClosedReason::NodeLeaseExpired => {
                    self.fence();
                    self.session = None;
                    self.authentication = None;
                    self.challenge_intent = None;
                    Err(Error::Expired)
                }
                ClosedReason::MembershipRevoked => {
                    self.fence();
                    self.session = None;
                    Err(Error::MembershipRevoked)
                }
                ClosedReason::ServerShutdown => Ok(false),
            },
            StreamEvent::Reset { .. }
            | StreamEvent::Ready { .. }
            | StreamEvent::EntitiesDelta { .. }
            | StreamEvent::ConfigurationChanged { .. }
            | StreamEvent::ResourcesChanged { .. }
            | StreamEvent::ObservationStatus { .. }
            | StreamEvent::ObservationRemoved { .. } => Ok(true),
            // Initial projections are deliberately discarded: Ready triggers a
            // validated atomic GET/refresh, rather than installing an SSE cache.
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closure_reasons_and_transport_failure_have_distinct_authority_effects() {
        for (reason, expected) in [
            (ClosedReason::SessionExpired, "expired"),
            (ClosedReason::NodeLeaseExpired, "expired"),
            (ClosedReason::MembershipRevoked, "revoked"),
            (ClosedReason::ServerShutdown, "reconnect"),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let mut control = Control::open(
                &directory.path().canonicalize().unwrap().join("node"),
                "http://127.0.0.1:9",
                reqwest::Client::builder().no_proxy(),
            )
            .unwrap();
            let scope = control.events_cancel.clone();
            assert!(
                !control
                    .receive_event(EventNotice {
                        owner: Arc::new(()),
                        scope: scope.clone(),
                        result: Ok(NodeEvent(StreamEvent::Closed {
                            reason: ClosedReason::MembershipRevoked
                        })),
                    })
                    .unwrap()
            );
            assert!(!scope.is_cancelled());
            let result = control.receive_event(EventNotice {
                owner: control.application_owner.clone(),
                scope: scope.clone(),
                result: Ok(NodeEvent(StreamEvent::Closed { reason })),
            });
            match expected {
                "expired" => assert!(matches!(result, Err(Error::Expired))),
                "revoked" => assert!(matches!(result, Err(Error::MembershipRevoked))),
                _ => assert!(matches!(result, Ok(false))),
            }
            assert_eq!(scope.is_cancelled(), expected != "reconnect");
            for error in [
                ClientError::Network { status: None },
                ClientError::Decode { status: 200 },
                ClientError::BodyLimit { status: 200 },
                ClientError::Http {
                    status: 503,
                    problem: None,
                },
            ] {
                let scope = control.events_cancel.clone();
                assert!(
                    !control
                        .receive_event(EventNotice {
                            owner: control.application_owner.clone(),
                            scope: scope.clone(),
                            result: Err(error)
                        })
                        .unwrap()
                );
                assert!(!scope.is_cancelled());
            }
        }
    }
}
