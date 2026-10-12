//! Routing profile selection. The Server owns the persistent selection and
//! decides the applied profile; Control only asks for a change under the
//! held netmap's selection version and keeps nothing of the answer: the
//! Node's next netmap carries the new selection.
use crate::{Control, Error, exchange::Prepared, exchange::Response};
use meshway_configuration::selection::RoutingSelectionState;
use meshway_control_client::ClientError;
use meshway_protocol::{RoutingSelectionRequest, RoutingSelectionResponse};

impl Control {
    /// Only under a live lease: the compare-and-swap base is the selection
    /// version this Node's held netmap carries.
    pub(crate) fn prepare_select_profile(
        &mut self,
        profile: String,
    ) -> Result<Prepared<(), RoutingSelectionResponse>, Error> {
        let instance = self.current_instance()?;
        let lease = self.netmap_lease().ok_or(Error::Expired)?;
        let netmap = self.held_netmap(lease.netmap)?;
        let body = RoutingSelectionRequest {
            instance,
            expected_version: netmap
                .netmap
                .projection
                .selection_versions
                .get(&netmap.netmap.node_id)
                .copied(),
            profile,
        };
        self.prepare((), move |client, token| async move {
            client.select_node_routing_profile(&body, &token).await
        })
    }

    /// A decoded refusal of the profile or of a stale version leaves the
    /// session and instance as they are; anything else is classified as for
    /// every other request.
    pub(crate) fn apply_select_profile(
        &mut self,
        (): (),
        response: Response<RoutingSelectionResponse>,
    ) -> Result<RoutingSelectionState, Error> {
        match response {
            Err(ClientError::Http {
                status: status @ (409 | 422),
                problem: Some(body),
            }) if body.status == status
                && matches!(
                    body.problem,
                    meshway_protocol::Problem::VersionConflict { .. }
                        | meshway_protocol::Problem::RoutingProfileUnavailable { .. }
                ) =>
            {
                Err(Error::Server(ClientError::Http {
                    status,
                    problem: Some(body),
                }))
            }
            other => self.registered(other).map(|answer| answer.selection),
        }
    }
}
