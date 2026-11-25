use super::DomainRoute;
use std::collections::{BTreeMap, BTreeSet};

fn authority(value: &str) -> &str {
    value
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap_or_default()
}

fn endpoint(value: &str) -> Option<(String, u16)> {
    let (name, port) = authority(value).rsplit_once(':')?;
    let port: u16 = port.parse().ok()?;
    (port > 0
        && !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".:-_[]".contains(&byte)))
    .then(|| (name.into(), port))
}

fn domain(value: &str) -> Option<String> {
    let name = authority(value.trim_end_matches(',')).split(':').next()?;
    (!name.is_empty()
        && name != "_"
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".*-".contains(&byte)))
    .then(|| name.to_owned())
}

#[derive(Default)]
struct Site {
    names: Vec<String>,
    targets: Vec<String>,
}

pub(super) fn parse(configs: &[(String, String)]) -> Vec<DomainRoute> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut sites = Vec::new();
    for (proxy, text) in configs
        .iter()
        .filter(|(kind, _)| matches!(kind.as_str(), "nginx" | "caddy"))
    {
        let mut stack: Vec<Vec<String>> = Vec::new();
        let mut site = Site::default();
        let mut words: Vec<String> = Vec::new();
        for line in text.lines() {
            let line = line
                .split('#')
                .next()
                .unwrap_or_default()
                .replace('{', " { ")
                .replace('}', " } ")
                .replace(';', " ; ");
            for token in line
                .split_whitespace()
                .chain((proxy == "caddy").then_some(";"))
            {
                match token {
                    "{" => {
                        if stack.is_empty() && proxy == "caddy" {
                            site.names = words.iter().filter_map(|value| domain(value)).collect();
                        } else if proxy == "caddy"
                            && words.first().is_some_and(|word| word == "reverse_proxy")
                        {
                            site.targets.extend_from_slice(&words[1..]);
                        }
                        stack.push(std::mem::take(&mut words));
                    }
                    ";" | "}" => {
                        let keyword = words.first().map(String::as_str);
                        if let Some(values) = words.get(1..) {
                            if proxy == "caddy"
                                && !stack.is_empty()
                                && keyword == Some("reverse_proxy")
                            {
                                site.targets.extend_from_slice(values);
                            }
                            if proxy == "nginx" {
                                if let Some(group) = stack
                                    .last()
                                    .filter(|block| {
                                        block.first().is_some_and(|key| key == "upstream")
                                    })
                                    .and_then(|block| block.get(1))
                                    && keyword == Some("server")
                                    && let Some(target) = values.first()
                                {
                                    groups
                                        .entry(group.clone())
                                        .or_default()
                                        .push(target.clone());
                                }
                                if stack.iter().any(|block| block.as_slice() == ["server"]) {
                                    match keyword {
                                        Some("server_name") => site.names.extend(
                                            values.iter().filter_map(|value| domain(value)),
                                        ),
                                        Some("proxy_pass") => {
                                            site.targets.extend_from_slice(values)
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        words.clear();
                        if token == "}"
                            && let Some(block) = stack.pop()
                            && ((proxy == "nginx" && block.as_slice() == ["server"])
                                || (proxy == "caddy" && stack.is_empty()))
                        {
                            sites.push((proxy.clone(), std::mem::take(&mut site)));
                        }
                    }
                    other => words.push(other.trim_matches(['\'', '"']).into()),
                }
            }
        }
    }
    let mut found = BTreeSet::new();
    for (proxy, site) in sites {
        for target in site.targets {
            let servers = if proxy == "nginx" {
                groups.get(authority(&target))
            } else {
                None
            };
            for (upstream, port) in servers
                .map(Vec::as_slice)
                .unwrap_or(std::slice::from_ref(&target))
                .iter()
                .filter_map(|value| endpoint(value))
            {
                for name in &site.names {
                    found.insert(DomainRoute {
                        proxy: proxy.clone(),
                        domain: name.clone(),
                        upstream: upstream.clone(),
                        port,
                    });
                    if found.len() >= 4096 {
                        return found.into_iter().collect();
                    }
                }
            }
        }
    }
    found.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caddy_transport_block_keeps_upstream_but_not_secrets() {
        let routes = parse(&[(
            "caddy".into(),
            "a.test {\nreverse_proxy localhost:9000 {\nheader_up Authorization secret\n}\n}\n"
                .into(),
        )]);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].port, 9000);
    }

    #[test]
    fn scopes_sites_and_resolves_static_upstreams() {
        let found = parse(&[("nginx".into(), "http { upstream app { server localhost:9000; } server { location / { proxy_pass http://app; } server_name a.test; } server { server_name other.test; } }".into()), ("caddy".into(), "https://b.test {\n reverse_proxy [::1]:3000\n}\n".into())]);
        assert_eq!(found.len(), 2);
        assert!(
            found
                .iter()
                .any(|route| route.domain == "a.test" && route.port == 9000)
        );
        assert!(
            found
                .iter()
                .any(|route| route.domain == "b.test" && route.port == 3000)
        );
    }
}
