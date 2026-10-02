//! Ordered Mesh rule diagnostics over the current node/tag inventory.
use super::*;

pub(super) fn lint(rules: &[MeshRule], nodes: &[&Node]) -> Vec<MeshLint> {
    if rules.is_empty() {
        return vec![];
    }
    let mut totals = vec![0; rules.len()];
    let mut shadowed = vec![0; rules.len()];
    let mut examples = vec![None; rules.len()];
    for (src, a) in nodes.iter().enumerate() {
        for (dst, b) in nodes.iter().enumerate() {
            if src == dst {
                continue;
            }
            let mut winner = None;
            for (i, rule) in rules.iter().enumerate() {
                if !matches(&rule.src, a) || !matches(&rule.dst, b) {
                    continue;
                }
                totals[i] += 1;
                if let Some(winning_rule) = winner {
                    shadowed[i] += 1;
                    examples[i].get_or_insert_with(|| {
                        let winning: &MeshRule = &rules[winning_rule];
                        MeshShadowExample {
                            src: src as u16,
                            dst: dst as u16,
                            winning_rule_id: winning.id.clone(),
                            winning_rule: winning_rule as u16,
                            winning_via: winning.via.clone(),
                            shadowed_via: rule.via.clone(),
                        }
                    });
                } else {
                    winner = Some(i);
                }
            }
        }
    }
    rules
        .iter()
        .enumerate()
        .filter_map(|(i, rule)| {
            examples[i].take().map(|example| MeshLint {
                rule_id: rule.id.clone(),
                rule: i as u16,
                shadowed_flows: shadowed[i],
                total_flows: totals[i],
                fully_shadowed: shadowed[i] == totals[i],
                inventory_dependent: true,
                example,
            })
        })
        .collect()
}
