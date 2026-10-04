use crate::SubjectContext;
use meshway_configuration::*;
use meshway_model::NodeId;
use std::collections::{BTreeMap, BTreeSet};

mod references;
mod routing;
pub use references::{domain_name_matches, domain_suffix_matches, topology_allows};
type Check = ValidationReport;

fn pointer(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

struct PolicySemantics;
impl ConfigurationSemantics for PolicySemantics {
    fn access_and_topology(
        &self,
        configuration: &mut NetworkConfiguration,
        context: &ValidationContext<'_>,
        report: &mut ValidationReport,
    ) {
        references::rules(configuration, context, report);
    }
    fn routing(
        &self,
        configuration: &mut NetworkConfiguration,
        context: &ValidationContext<'_>,
        report: &mut ValidationReport,
    ) {
        routing::validate(configuration, context, report);
    }
}

/// Full configuration normalization and semantic validation. No I/O, material
/// generation, authentication, or publication occurs here. All callers share
/// the same hooks inside configuration's normalization/error ordering.
pub fn normalize_with_dependencies(
    input: NetworkConfigurationInput,
    context: &ValidationContext<'_>,
) -> Result<NormalizedConfiguration, ConfigurationError> {
    normalize_with_semantics(input, context, &PolicySemantics)
}

pub fn normalize(
    input: NetworkConfigurationInput,
    context: &ValidationContext<'_>,
) -> Result<(NetworkConfiguration, BTreeMap<String, UserBinding>), ConfigurationError> {
    let result = normalize_with_dependencies(input, context)?;
    Ok((result.configuration, result.user_bindings))
}

pub fn plan_configuration(
    input: NetworkConfigurationInput,
    context: &ValidationContext<'_>,
) -> Result<ConfigurationPlan, ConfigurationError> {
    plan_with_semantics(input, context, &PolicySemantics)
}

pub fn validate_configuration(
    configuration: &NetworkConfiguration,
    context: &ValidationContext<'_>,
) -> Result<BTreeMap<String, UserBinding>, ConfigurationError> {
    normalize(NetworkConfigurationInput::from(configuration), context).map(|(_, bindings)| bindings)
}

pub fn parse_document(
    source: String,
    context: &ValidationContext<'_>,
) -> Result<ConfigurationDocument, ConfigurationError> {
    let (configuration, _) = normalize(parse_jsonc(&source)?, context)?;
    Ok(ConfigurationDocument::new(source, configuration))
}
