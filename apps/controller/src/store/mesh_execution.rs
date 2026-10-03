//! Runtime-bound execution capability negotiation and saved-only receipts.
//! Source policy ownership is network_policy; coherent projection is publication.
use super::{NetworkId, Store, configuration, network_policy, publication};
use crate::Error;
use meshway_protocol::*;
use rusqlite::{Connection, TransactionBehavior, params};

fn check_epoch(
    db: &Connection,
    network: &NetworkId,
    id: &str,
    request: &MeshExecutionRequest,
) -> Result<(), Error> {
    if request.epoch == 0 || !local::valid_credential(&request.instance_id) {
        return Err(Error::Invalid("invalid execution runtime binding"));
    }
    if !db.query_row("SELECT EXISTS(SELECT 1 FROM connection_runtime_leases WHERE network_id=?1 AND node_id=?2 AND epoch=?3 AND instance_id=?4)",params![network.as_str(),id,request.epoch,request.instance_id],|r|r.get::<_,bool>(0))? {
        return Err(Error::Conflict);
    }
    Ok(())
}

impl Store {
    pub fn mesh_execution(
        &mut self,
        token: &str,
        request: &MeshExecutionRequest,
    ) -> Result<MeshExecutionResponse, Error> {
        let network = self.node_network(token)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = super::connection_runtime::bound_node(&tx, &network, token)?;
        check_epoch(&tx, &network, &id, request)?;
        if request.capabilities != [MeshExecutionCapability::Ipv4StatefulExactPathV2] {
            if tx.execute(
                "DELETE FROM mesh_execution_capabilities WHERE network_id=?1 AND node_id=?2",
                params![network.as_str(), id],
            )? != 0
            {
                configuration::bump(&tx, &network)?;
                publication::finish_change(&tx, &network, publication::ChangeKind::Adjacency)?;
            }
            tx.commit()?;
            return Err(Error::Unprocessable(
                "IPv4 stateful exact-path execution v2 capability required",
            ));
        }
        let changed= !tx.query_row("SELECT EXISTS(SELECT 1 FROM mesh_execution_capabilities WHERE network_id=?1 AND node_id=?2 AND epoch=?3 AND instance_id=?4)",params![network.as_str(),id,request.epoch,request.instance_id],|r|r.get::<_,bool>(0))?;
        if changed {
            tx.execute("INSERT INTO mesh_execution_capabilities(node_id,network_id,epoch,instance_id,capability) VALUES(?1,?2,?3,?4,2) ON CONFLICT(node_id) DO UPDATE SET epoch=excluded.epoch,instance_id=excluded.instance_id,capability=excluded.capability WHERE network_id=excluded.network_id",params![id,network.as_str(),request.epoch,request.instance_id])?;
            configuration::bump(&tx, &network)?;
            publication::finish_change(&tx, &network, publication::ChangeKind::Adjacency)?;
        }
        let revision = configuration::revision(&tx, &network)?;
        let (policy, _) = network_policy::source(&tx, &network)?;
        let snapshot = publication::mesh_execution::Publication::read(&tx, &network, &policy)?
            .project(&id, &self.material_key)?;
        let response = MeshExecutionResponse { revision, snapshot };
        tx.commit()?;
        Ok(response)
    }

    pub fn acknowledge_mesh_execution(
        &mut self,
        token: &str,
        request: &AcknowledgeMeshExecutionRequest,
    ) -> Result<AcknowledgeMeshExecutionResponse, Error> {
        let network = self.node_network(token)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = super::connection_runtime::bound_node(&tx, &network, token)?;
        check_epoch(&tx, &network, &id, &request.runtime)?;
        if request.runtime.capabilities != [MeshExecutionCapability::Ipv4StatefulExactPathV2] {
            return Err(Error::Conflict);
        }
        let (policy, _) = network_policy::source(&tx, &network)?;
        let current = publication::mesh_execution::Publication::read(&tx, &network, &policy)?
            .project(&id, &self.material_key)?;
        if request.revision != current.payload.revision || request.digest != current.digest {
            return Err(Error::Conflict);
        }
        tx.execute("INSERT INTO mesh_execution_receipts(node_id,network_id,epoch,revision,digest) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(node_id) DO UPDATE SET epoch=excluded.epoch,revision=excluded.revision,digest=excluded.digest WHERE network_id=excluded.network_id",params![id,network.as_str(),request.runtime.epoch,request.revision,request.digest])?;
        tx.commit()?;
        Ok(AcknowledgeMeshExecutionResponse {
            revision: request.revision,
            digest: request.digest.clone(),
            saved: true,
        })
    }
}
