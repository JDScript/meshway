use super::*;

/// The State reference of a socket on the current content, or `None` while
/// no live instance qualifies the Node.
pub(in crate::manager) fn socket_reference(
    shared: &Shared,
    id: [u8; 32],
    resource: ResourceRef,
) -> Result<Option<SocketReference>, Error> {
    let Some(instance) = shared.instance.lock().map_err(|_| Error::Worker)?.clone() else {
        return Ok(None);
    };
    let plan = shared.plan();
    Ok(Some(SocketReference {
        node_id: plan.node,
        instance,
        netmap: plan.reference,
        socket_id: id.iter().map(|byte| format!("{byte:02x}")).collect(),
        socket_generation: shared.generation,
        resource,
    }))
}

/// The socket carries a selected Native UDP channel or an attempt.
pub(super) fn socket_active(
    shared: &Shared,
    pending: &BTreeMap<UdpTraversalId, Pending>,
    id: &[u8; 32],
) -> bool {
    pending.values().any(|value| value.endpoint == *id)
        || shared.links.lock().is_ok_and(|links| {
            links.values().any(|link| {
                link.fact.socket_id == *id
                    && link.fact.carrier.protocol == TransportProtocol::NativeUdp
                    && link.scope.check().is_ok()
            })
        })
}

pub(in crate::manager) fn matching_endpoint(
    shared: &Shared,
    offer: &ValidatedOffer,
) -> Option<[u8; 32]> {
    let sockets = shared.sockets.lock().ok()?;
    sockets.iter().find_map(|(id, fact)| {
        (fact.reference == offer.local().candidate.socket
            && socket_compatible(&shared.plan(), &offer.offer().carrier, fact)
            && fact.endpoints.iter().any(|observation| {
                fact.candidate(observation)
                    .is_ok_and(|candidate| candidate == offer.local().candidate)
                    && observation.address == offer.local().address
            }))
        .then_some(*id)
    })
}

pub(super) fn socket_compatible(plan: &Plan, carrier: &Carrier, fact: &SocketFact) -> bool {
    if plan.node == carrier.listener {
        return fact.reference.resource
            == (ResourceRef::Listener {
                key: carrier.listener_key.clone(),
            });
    }
    let key = format!("{}/{}", carrier.listener, carrier.listener_key);
    let Some(dial) = plan.dialers.get(&key) else {
        return false;
    };
    let meshway_transport::spec::DialParameters::NativeUdp { options } = &dial.parameters else {
        return false;
    };
    match &fact.reference.resource {
        ResourceRef::Dial { key: bound } => {
            *bound == key && dial_bind_matches(options, fact.bound_address)
        }
        ResourceRef::Listener { key } => plan
            .listeners
            .get(key)
            .is_some_and(|listener| listener_covers(listener, fact.bound_address, options)),
        ResourceRef::Coordination { .. } => false,
    }
}

pub(in crate::manager) fn listener_covers(
    listener: &meshway_transport::spec::PreparedListener,
    local: SocketAddr,
    options: &meshway_transport::spec::NativeUdpDialer,
) -> bool {
    dial_bind_matches(options, local)
        && matches!(listener.declaration(), meshway_transport::spec::Listener::NativeUdp(listener)
            if listener.enabled && listener.bind_interface.is_empty() && listener.socket == options.socket)
}

fn dial_bind_matches(
    options: &meshway_transport::spec::NativeUdpDialer,
    local: SocketAddr,
) -> bool {
    options.enabled
        && options.bind_interface.is_empty()
        && (options.local_address.is_empty()
            || options.local_address.parse::<std::net::IpAddr>().ok() == Some(local.ip()))
}
