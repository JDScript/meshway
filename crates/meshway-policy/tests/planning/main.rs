use meshway_configuration::{execution::*, *};
use meshway_model::state::{Capability, StreamKind, TransportRole};
use meshway_model::*;
use meshway_policy::{SubjectContext, compile::*, mesh_access_allows_any};
use meshway_transport_spec::*;
use std::collections::BTreeMap;
mod plan;

mod support;
use support::*;
mod discovery;
mod topology;
