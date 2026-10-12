//! Exit side of trusted DNS associations: completing the standing task the
//! held netmap's `ServeExitResolver` plan issued, one report per answered
//! query. The answer itself already went back to the source; acceptance
//! reaches both Nodes through their next sync.
use crate::{Control, Error, exchange::Prepared, exchange::Response};
use meshway_model::NetmapRef;
use meshway_protocol::{DnsAssociationAccepted, DnsAssociationReport};

impl Control {
    /// Only for the exact netmap this Node holds under a live lease.
    pub(crate) fn prepare_dns_association(
        &mut self,
        report: &DnsAssociationReport,
    ) -> Result<Prepared<NetmapRef, DnsAssociationAccepted>, Error> {
        self.held_netmap(report.netmap)?;
        let body = report.clone();
        self.prepare(report.netmap, move |client, token| async move {
            client.report_dns_association(&body, &token).await
        })
    }

    pub(crate) fn apply_dns_association(
        &mut self,
        netmap: NetmapRef,
        response: Response<DnsAssociationAccepted>,
    ) -> Result<DnsAssociationAccepted, Error> {
        let accepted = self.registered(response)?;
        self.held_netmap(netmap)?;
        Ok(accepted)
    }
}
