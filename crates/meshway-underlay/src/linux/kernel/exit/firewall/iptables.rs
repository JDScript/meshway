use super::{
    host::Failure,
    judge::{Hook, Match, Rule, Ruleset, Verdict},
};

pub(super) fn legacy_blocks(target: &str) -> bool {
    matches!(
        target,
        "DROP" | "REJECT" | "QUEUE" | "NFQUEUE" | "TPROXY" | "TEE"
    )
}

/// `iptables-legacy-save` (or `ip6tables-legacy-save`) output: its chains,
/// and whether any rule reads or writes the packet or connection mark or
/// conntrack handling.
#[derive(Debug, Default)]
pub(in crate::linux::kernel::exit) struct Legacy {
    pub(super) ruleset: Ruleset,
    pub marks: bool,
}

impl Legacy {
    /// `source` keys the chains: `legacy` (IPv4) or `legacy6` (IPv6).
    pub fn parse(text: &str, source: &str) -> Result<Self, Failure> {
        let mut legacy = Self::default();
        let mut table: Option<String> = None;
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('*') {
                table = Some(name.to_owned());
                continue;
            }
            if line == "COMMIT" {
                table = None;
                continue;
            }
            let table = table.as_deref().ok_or(Failure::Malformed)?;
            let key = |chain: &str| (source.to_owned(), table.to_owned(), chain.to_owned());
            if let Some(declaration) = line.strip_prefix(':') {
                let mut fields = declaration.split_whitespace();
                let name = fields.next().ok_or(Failure::Malformed)?;
                let policy = fields.next().ok_or(Failure::Malformed)?;
                let chain = legacy.ruleset.chains.entry(key(name)).or_default();
                chain.hook = match (table, name) {
                    ("raw" | "mangle" | "nat", "PREROUTING") => Some(Hook::Prerouting),
                    ("filter" | "mangle" | "security", "FORWARD") => Some(Hook::Forward),
                    ("filter" | "mangle" | "raw" | "nat" | "security", "OUTPUT") => {
                        Some(Hook::Output)
                    }
                    ("mangle" | "nat", "POSTROUTING") => Some(Hook::Postrouting),
                    _ => None,
                };
                chain.drops = policy == "DROP";
                continue;
            }
            let tokens = tokenize(line)?;
            let (Some("-A"), Some(chain)) = (
                tokens.first().map(String::as_str),
                tokens.get(1).map(String::as_str),
            ) else {
                return Err(Failure::Malformed);
            };
            let declared = |name: &str| legacy.ruleset.chains.contains_key(&key(name));
            let (rule, marks) = legacy_rule(&tokens[2..], &declared)?;
            legacy.marks |= marks;
            legacy
                .ruleset
                .chains
                .entry(key(chain))
                .or_default()
                .rules
                .push(rule);
        }
        if table.is_some() || legacy.ruleset.chains.len() > 16384 {
            return Err(Failure::Malformed);
        }
        Ok(legacy)
    }
}

fn tokenize(line: &str) -> Result<Vec<String>, Failure> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = line.chars();
    while let Some(character) = chars.next() {
        match character {
            '"' => quoted = !quoted,
            '\\' if quoted => current.push(chars.next().ok_or(Failure::Malformed)?),
            character if character.is_whitespace() && !quoted => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            character => current.push(character),
        }
    }
    if quoted || tokens.len() > 512 {
        return Err(Failure::Malformed);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

/// One `-A` rule; `declared` names this table's chains, any other target is
/// an extension.
fn legacy_rule(
    tokens: &[String],
    declared: &dyn Fn(&str) -> bool,
) -> Result<(Rule, bool), Failure> {
    let mut matches = Vec::new();
    let mut verdict = Verdict::Continue;
    let mut marks = false;
    let mut negated = false;
    let mut index = 0;
    let value = |index: usize| tokens.get(index).cloned().ok_or(Failure::Malformed);
    while index < tokens.len() {
        let token = tokens[index].as_str();
        match token {
            "!" => {
                negated = true;
                index += 1;
                continue;
            }
            "-i" | "--in-interface" => {
                matches.push(Match::Input(!negated, vec![value(index + 1)?]))
            }
            "-o" | "--out-interface" => {
                matches.push(Match::Output(!negated, vec![value(index + 1)?]))
            }
            "--ctstate" | "--state" => matches.push(Match::State(
                !negated,
                value(index + 1)?
                    .split(',')
                    .map(str::to_ascii_lowercase)
                    .collect(),
            )),
            "-m" | "--match" => {
                let module = value(index + 1)?;
                marks |= matches!(module.as_str(), "mark" | "connmark");
                // `-m rpfilter [--invert] [--loose] [--validmark]
                // [--accept-local]`: a reverse-path filter, satisfied.
                if module == "rpfilter" {
                    matches.push(Match::ReversePath);
                    index += 2;
                    while matches!(
                        tokens.get(index).map(String::as_str),
                        Some("--invert" | "--loose" | "--validmark" | "--accept-local")
                    ) {
                        index += 1;
                    }
                    negated = false;
                    continue;
                }
            }
            "-j" | "--jump" | "-g" | "--goto" => {
                let target = value(index + 1)?;
                marks |= matches!(
                    target.as_str(),
                    "MARK" | "CONNMARK" | "CT" | "NOTRACK" | "TPROXY"
                );
                verdict = match target.as_str() {
                    "ACCEPT" => Verdict::Accept,
                    "RETURN" => Verdict::Return,
                    target if legacy_blocks(target) => Verdict::Block,
                    target if !declared(target) => Verdict::Continue,
                    target if matches!(token, "-g" | "--goto") => Verdict::Goto(target.to_owned()),
                    target => Verdict::Jump(target.to_owned()),
                };
            }
            "--comment" => {}
            "-d" | "--destination" => matches.push(
                value(index + 1)?
                    .split(',')
                    .map(|net| {
                        net.parse::<ipnet::IpNet>()
                            .ok()
                            .or_else(|| net.parse::<std::net::IpAddr>().ok().map(Into::into))
                    })
                    .collect::<Option<Vec<_>>>()
                    .map_or(Match::Unknown, |nets| Match::Destination(!negated, nets)),
            ),
            "-p" | "--protocol" | "-s" | "--source" | "-f" | "--fragment" => {
                matches.push(Match::Unknown)
            }
            option if option.starts_with('-') && !matches!(option, "-c") => {
                matches.push(Match::Unknown)
            }
            _ => {}
        }
        negated = false;
        index += 1;
    }
    Ok((Rule { matches, verdict }, marks))
}

#[cfg(test)]
mod tests {
    use super::super::tests::crossing;
    use super::*;
    use meshway_model::exit::{ExitFamily, ExitUnavailableReason as Reason};

    /// Docker's raw protection of a published container (`-d <container>
    /// ! -i <bridge> -j DROP`) excludes every exit destination.
    #[test]
    fn legacy_prerouting_drops_are_judged_by_destination() {
        let docker = "*raw\n:PREROUTING ACCEPT [0:0]\n:OUTPUT ACCEPT [0:0]\n\
            -A PREROUTING -d 172.17.0.2/32 ! -i docker0 -j DROP\nCOMMIT\n";
        assert_eq!(
            Legacy::parse(docker, "legacy")
                .unwrap()
                .ruleset
                .judge(&crossing(), ExitFamily::Ipv4),
            Ok(())
        );
        let all = "*raw\n:PREROUTING ACCEPT [0:0]\n-A PREROUTING ! -i docker0 -j DROP\nCOMMIT\n";
        assert_eq!(
            Legacy::parse(all, "legacy")
                .unwrap()
                .ruleset
                .judge(&crossing(), ExitFamily::Ipv4),
            Err(Reason::FirewallConflict)
        );
        let policy = "*mangle\n:PREROUTING DROP [0:0]\nCOMMIT\n";
        assert_eq!(
            Legacy::parse(policy, "legacy")
                .unwrap()
                .ruleset
                .judge(&crossing(), ExitFamily::Ipv4),
            Err(Reason::FirewallConflict)
        );
    }

    #[test]
    fn legacy_chains_are_judged_by_content() {
        let docker = "*nat\n:PREROUTING ACCEPT [0:0]\n:POSTROUTING ACCEPT [0:0]\n:DOCKER - [0:0]\n\
            -A POSTROUTING -s 172.17.0.0/16 ! -o docker0 -j MASQUERADE\nCOMMIT\n\
            *filter\n:INPUT ACCEPT [0:0]\n:FORWARD ACCEPT [0:0]\n:OUTPUT ACCEPT [0:0]\n\
            :DOCKER - [0:0]\n:DOCKER-ISOLATION-STAGE-1 - [0:0]\n:DOCKER-ISOLATION-STAGE-2 - [0:0]\n:DOCKER-USER - [0:0]\n\
            -A FORWARD -j DOCKER-USER\n-A FORWARD -j DOCKER-ISOLATION-STAGE-1\n\
            -A FORWARD -o docker0 -m conntrack --ctstate RELATED,ESTABLISHED -j ACCEPT\n\
            -A FORWARD -o docker0 -j DOCKER\n-A FORWARD -i docker0 ! -o docker0 -j ACCEPT\n\
            -A DOCKER-ISOLATION-STAGE-1 -i docker0 ! -o docker0 -j DOCKER-ISOLATION-STAGE-2\n\
            -A DOCKER-ISOLATION-STAGE-1 -j RETURN\n-A DOCKER-ISOLATION-STAGE-2 -o docker0 -j DROP\n\
            -A DOCKER-ISOLATION-STAGE-2 -j RETURN\n-A DOCKER-USER -j RETURN\n\
            -A DOCKER -d 172.17.0.2/32 ! -i docker0 -o docker0 -p tcp -m tcp --dport 80 -j ACCEPT\nCOMMIT\n";
        let legacy = Legacy::parse(docker, "legacy").unwrap();
        assert!(!legacy.marks);
        assert_eq!(legacy.ruleset.judge(&crossing(), ExitFamily::Ipv4), Ok(()));
        let dropping = Legacy::parse(
            &docker.replace(":FORWARD ACCEPT", ":FORWARD DROP"),
            "legacy",
        )
        .unwrap();
        assert_eq!(
            dropping.ruleset.judge(&crossing(), ExitFamily::Ipv4),
            Err(Reason::FirewallConflict)
        );
        let ufw = "*filter\n:FORWARD DROP [0:0]\n:ufw-user-forward - [0:0]\n\
            -A FORWARD -j ufw-user-forward\n-A ufw-user-forward -i eth0 -o eth0 -j ACCEPT\n\
            -A ufw-user-forward -m comment --comment \"a \\\"quoted\\\" note\" -j RETURN\nCOMMIT\n";
        assert_eq!(
            Legacy::parse(ufw, "legacy")
                .unwrap()
                .ruleset
                .judge(&crossing(), ExitFamily::Ipv4),
            Err(Reason::FirewallConflict)
        );
        let marking = "*mangle\n:PREROUTING ACCEPT [0:0]\n-A PREROUTING -i eth1 -j MARK --set-xmark 0x1/0xffffffff\nCOMMIT\n";
        assert!(Legacy::parse(marking, "legacy").unwrap().marks);
        for malformed in [
            "-A FORWARD -j DROP\n",
            "*filter\n-A FORWARD -i\nCOMMIT\n",
            "*filter\n",
        ] {
            assert!(Legacy::parse(malformed, "legacy").is_err(), "{malformed:?}");
        }
    }
}
