use meshway_model::RequestId;
use meshway_protocol::configuration::{
    ConfigurationSchemaResponse, TransportCatalogEntry, TransportCatalogResponse,
};
use meshway_transport_spec::*;
use utoipa::{PartialSchema, ToSchema};

pub(crate) fn schema(id: RequestId) -> Result<ConfigurationSchemaResponse, super::Error> {
    let mut definitions = Vec::new();
    meshway_configuration::NetworkConfigurationInput::schemas(&mut definitions);
    definitions.push((
        meshway_configuration::NetworkConfigurationInput::name().into_owned(),
        meshway_configuration::NetworkConfigurationInput::schema(),
    ));
    let definitions: std::collections::BTreeMap<_, _> = definitions.into_iter().collect();
    let mut document = serde_json::json!({"$schema":"https://json-schema.org/draft/2020-12/schema","$ref":"#/$defs/NetworkConfigurationInput","$defs":definitions});
    fn local_refs(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(serde_json::Value::String(reference)) = object.get_mut("$ref")
                    && let Some(name) = reference.strip_prefix("#/components/schemas/")
                {
                    *reference = format!("#/$defs/{name}");
                }
                for value in object.values_mut() {
                    local_refs(value);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    local_refs(value);
                }
            }
            _ => {}
        }
    }
    local_refs(&mut document);
    Ok(ConfigurationSchemaResponse {
        request_id: id,
        schema_json: serde_json::to_string(&document)
            .map_err(|_| super::Error::encode("configuration schema"))?,
    })
}
pub(crate) fn transports(id: RequestId) -> TransportCatalogResponse {
    fn entry<L: ToSchema, D: ToSchema>(
        protocol: TransportProtocol,
        outer_socket: OuterSocket,
        capability: &str,
    ) -> TransportCatalogEntry {
        TransportCatalogEntry {
            protocol,
            outer_socket,
            listener_schema: L::name().into_owned(),
            dialer_schema: D::name().into_owned(),
            required_capability: capability.into(),
        }
    }
    TransportCatalogResponse {
        request_id: id,
        items: vec![
            entry::<NativeUdpListener, NativeUdpDialer>(
                TransportProtocol::NativeUdp,
                OuterSocket::Udp,
                "transport:native_udp",
            ),
            entry::<NativeTcpListener, NativeTcpDialer>(
                TransportProtocol::NativeTcp,
                OuterSocket::Tcp,
                "transport:native_tcp",
            ),
            entry::<VlessListener, VlessDialer>(
                TransportProtocol::Vless,
                OuterSocket::Tcp,
                "transport:vless",
            ),
            entry::<TrojanListener, TrojanDialer>(
                TransportProtocol::Trojan,
                OuterSocket::Tcp,
                "transport:trojan",
            ),
        ],
    }
}
