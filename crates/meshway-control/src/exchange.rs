//! Server requests prepared and applied under the Control lock and sent
//! without it, so that a sync answer (an installation, renewal or
//! withdrawal) never waits behind another request's network round trip.
use crate::{Control, Error};
use meshway_control_client::{Client, ClientError};
use meshway_protocol::ErrorResponse;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, atomic::AtomicBool},
};
use zeroize::Zeroizing;

pub(crate) type Response<T> = Result<T, ClientError<ErrorResponse>>;
type Send<T> = Pin<Box<dyn Future<Output = Response<T>> + std::marker::Send + 'static>>;

/// One prepared request: its owned send future and the state to apply its
/// answer with. It borrows nothing from Control.
pub(crate) struct Prepared<S, T> {
    state: S,
    send: Send<T>,
    context: Arc<AtomicBool>,
}

impl<S, T> Prepared<S, T> {
    pub(crate) fn with_state<U>(self, state: U) -> Prepared<U, T> {
        Prepared {
            state,
            send: self.send,
            context: self.context,
        }
    }
}

impl Control {
    /// Prepare `call` with an owned client and session token.
    pub(crate) fn prepare<S, T, F>(
        &self,
        state: S,
        call: impl FnOnce(Client, Zeroizing<String>) -> F,
    ) -> Result<Prepared<S, T>, Error>
    where
        F: Future<Output = Response<T>> + std::marker::Send + 'static,
    {
        let token = Zeroizing::new(self.token()?.to_owned());
        Ok(self.prepare_public(state, call(self.client.clone(), token)))
    }

    /// Prepare a request that needs no session token.
    pub(crate) fn prepare_public<S, T>(
        &self,
        state: S,
        send: impl Future<Output = Response<T>> + std::marker::Send + 'static,
    ) -> Prepared<S, T> {
        Prepared {
            state,
            send: Box::pin(send),
            context: Arc::clone(&self.context_validity),
        }
    }

    /// The answer of a request prepared before the session/instance context
    /// changed (fence, stop, re-authentication) is not applied.
    fn same_context(&self, context: &Arc<AtomicBool>) -> Result<(), Error> {
        if Arc::ptr_eq(context, &self.context_validity) {
            Ok(())
        } else {
            Err(Error::Pending)
        }
    }

    /// Send and apply while holding `&mut self` (no other owner shares it).
    pub(crate) async fn exchange<S, T, R>(
        &mut self,
        prepared: Prepared<S, T>,
        apply: impl FnOnce(&mut Self, S, Response<T>) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let Prepared {
            state,
            send,
            context,
        } = prepared;
        let response = send.await;
        self.same_context(&context)?;
        apply(self, state, response)
    }
}

/// Run one request against the shared Control: `prepare` and `apply` take
/// the lock; the HTTP exchange between them does not.
pub(crate) async fn unlocked<S, T, R>(
    control: &tokio::sync::Mutex<Control>,
    prepare: impl FnOnce(&mut Control) -> Result<Prepared<S, T>, Error>,
    apply: impl FnOnce(&mut Control, S, Response<T>) -> Result<R, Error>,
) -> Result<R, Error> {
    let Prepared {
        state,
        send,
        context,
    } = prepare(&mut *control.lock().await)?;
    let response = send.await;
    let mut control = control.lock().await;
    control.same_context(&context)?;
    apply(&mut control, state, response)
}

/// Operations of a Control shared behind a lock: each holds the lock only to
/// prepare its request and to apply the answer, never across the request.
pub mod shared {
    use super::unlocked;
    use crate::{ApplicationTicket, Control, Error, instance::Acquire, runtime::Receipt};
    use meshway_model::{NetmapRef, NodeId, state::Capability, state::NodeState};
    use meshway_protocol::{
        InstanceLease, ObservationList, ObservationResult, ObservationTask, StateAccepted,
        StateView,
        connectivity::{CancelUdpTraversalResponse, PrepareUdpTraversalRequest},
        execution::ApplyFailure,
    };
    use std::collections::BTreeMap;
    use tokio::sync::Mutex;

    use crate::connectivity::{AttemptView, Discovery, DiscoveryContext, Outcome, ValidatedOffer};

    pub async fn authenticate(control: &Mutex<Control>) -> Result<(), Error> {
        {
            let mut control = control.lock().await;
            if control.leaving().is_some() {
                return Err(Error::Pending);
            }
            if control.authentication_current()? {
                return Ok(());
            }
        }
        let session = unlocked(
            control,
            Control::prepare_challenge,
            Control::apply_challenge,
        )
        .await?;
        unlocked(control, |_| Ok(session), Control::apply_session).await
    }

    pub async fn acquire_instance(
        control: &Mutex<Control>,
        capabilities: BTreeMap<String, Capability>,
    ) -> Result<InstanceLease, Error> {
        let prepared = match control.lock().await.prepare_acquire(capabilities)? {
            Acquire::Held(lease) => return Ok(lease),
            Acquire::Request(prepared) => prepared,
        };
        unlocked(control, |_| Ok(prepared), Control::apply_acquire).await
    }

    pub async fn renew_instance(control: &Mutex<Control>) -> Result<InstanceLease, Error> {
        unlocked(control, Control::prepare_renew, Control::apply_renew).await
    }

    pub async fn report_runtime_state(
        control: &Mutex<Control>,
        state: NodeState,
        observed_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<StateAccepted, Error> {
        unlocked(
            control,
            |control| control.prepare_runtime_report(state, observed_at),
            Control::apply_state_report,
        )
        .await
    }

    pub async fn retry_state_report(control: &Mutex<Control>) -> Result<StateAccepted, Error> {
        unlocked(
            control,
            Control::prepare_state_report,
            Control::apply_state_report,
        )
        .await
    }

    pub async fn begin_application(
        control: &Mutex<Control>,
        netmap: NetmapRef,
    ) -> Result<ApplicationTicket, Error> {
        let prepared = match control.lock().await.prepare_begin_application(netmap)? {
            Receipt::Done(ticket) => return Ok(ticket),
            Receipt::Send(prepared) => prepared,
        };
        unlocked(control, |_| Ok(prepared), Control::apply_saved).await
    }

    pub async fn finish_application(
        control: &Mutex<Control>,
        ticket: &ApplicationTicket,
        result: Result<(), ApplyFailure>,
        observed_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), Error> {
        unlocked(
            control,
            |control| control.prepare_completion(ticket, result, observed_at),
            Control::apply_completion,
        )
        .await
    }

    pub async fn get_observation_tasks(
        control: &Mutex<Control>,
        netmap: NetmapRef,
    ) -> Result<ObservationList, Error> {
        unlocked(
            control,
            |control| control.prepare_observation_tasks(netmap),
            Control::apply_observation_tasks,
        )
        .await
    }

    pub async fn report_observation_result(
        control: &Mutex<Control>,
        task: &ObservationTask,
        result: &ObservationResult,
    ) -> Result<ObservationTask, Error> {
        unlocked(
            control,
            |control| control.prepare_observation_result(task, result),
            Control::apply_observation_result,
        )
        .await
    }

    /// Complete one DNS association task of the held netmap (exit resolver).
    pub async fn report_dns_association(
        control: &Mutex<Control>,
        report: &meshway_protocol::DnsAssociationReport,
    ) -> Result<meshway_protocol::DnsAssociationAccepted, Error> {
        unlocked(
            control,
            |control| control.prepare_dns_association(report),
            Control::apply_dns_association,
        )
        .await
    }

    /// Ask the Server to select this Node's routing profile; the answer is
    /// its committed selection, which the next netmap carries.
    pub async fn select_profile(
        control: &Mutex<Control>,
        profile: String,
    ) -> Result<meshway_configuration::selection::RoutingSelectionState, Error> {
        unlocked(
            control,
            |control| control.prepare_select_profile(profile),
            Control::apply_select_profile,
        )
        .await
    }

    /// Ask the Server to clear this Node's routing selection so that the
    /// assignment's default applies; the answer is as for [`select_profile`].
    pub async fn clear_profile(
        control: &Mutex<Control>,
    ) -> Result<meshway_configuration::selection::RoutingSelectionState, Error> {
        unlocked(
            control,
            Control::prepare_clear_profile,
            Control::apply_select_profile,
        )
        .await
    }

    pub async fn query_udp_traversal(
        control: &Mutex<Control>,
        netmap: NetmapRef,
    ) -> Result<Outcome<Discovery>, Error> {
        unlocked(
            control,
            |control| control.prepare_query_udp_traversal(netmap),
            Control::apply_query_udp_traversal,
        )
        .await
    }

    pub async fn query_connectivity_peers(
        control: &Mutex<Control>,
        context: &DiscoveryContext,
        peers: &[NodeId],
    ) -> Result<StateView, Error> {
        unlocked(
            control,
            |control| {
                Ok(control
                    .prepare_connectivity_peers(context, peers)?
                    .into_prepared())
            },
            Control::apply_connectivity_peers,
        )
        .await
    }

    pub async fn prepare_udp_traversal(
        control: &Mutex<Control>,
        context: &DiscoveryContext,
        request: &PrepareUdpTraversalRequest,
    ) -> Result<Outcome<ValidatedOffer>, Error> {
        unlocked(
            control,
            |control| control.prepare_prepare_udp_traversal(context, request),
            Control::apply_prepare_udp_traversal,
        )
        .await
    }

    pub async fn accept_udp_traversal(
        control: &Mutex<Control>,
        offered: &ValidatedOffer,
    ) -> Result<Outcome<AttemptView>, Error> {
        unlocked(
            control,
            |control| control.prepare_accept_udp_traversal(offered),
            Control::apply_accept_udp_traversal,
        )
        .await
    }

    pub async fn cancel_udp_traversal(
        control: &Mutex<Control>,
        offered: &ValidatedOffer,
    ) -> Result<CancelUdpTraversalResponse, Error> {
        unlocked(
            control,
            |control| control.prepare_cancel_udp_traversal(offered),
            |control, (), response| control.registered(response),
        )
        .await
    }
}
