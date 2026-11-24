//! Rule matching (S2). User rules are order-independent: a matching `deny` wins over
//! `allow`, and `allow` (or a session grant) over `approve`. Calls no rule matches get
//! the built-in defaults.

use hostlane_protocol::{ClientInfo, Decision, HostEnv, PolicyRule, Risk, RuleConditions};

/// The facts about a call that rules are matched against.
#[derive(Debug, Clone)]
pub struct Subject {
    pub method: String,
    pub host: Option<String>,
    pub env: Option<HostEnv>,
    pub tags: Vec<String>,
    pub client: ClientInfo,
    pub command: Option<String>,
    /// The file a `files.*` call targets.
    pub path: Option<String>,
    /// Every remote path the call names or writes.
    pub paths: Vec<String>,
    pub risk: Risk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub decision: Decision,
    /// `rule <id>`, `session grant` or `default`.
    pub matched: String,
    /// Recorded as the approver when a rule or grant lets a call through.
    pub approver: Option<String>,
}

/// Calls from the desktop app are made by a person in the UI.
pub fn is_desktop(client: &ClientInfo) -> bool {
    client.name == "desktop"
}

/// Glob match: `*` is any text (including `/`), `?` one character, `\` escapes.
pub fn glob(pattern: &str, text: &str) -> bool {
    let mut tokens: Vec<(char, bool)> = Vec::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => tokens.push((chars.next().unwrap_or('\\'), true)),
            c => tokens.push((c, false)),
        }
    }
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match tokens.get(p) {
            Some(('*', false)) => {
                star = Some((p, t));
                p += 1;
            }
            Some(('?', false)) => {
                p += 1;
                t += 1;
            }
            Some((c, _)) if *c == text[t] => {
                p += 1;
                t += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    p = sp + 1;
                    t = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    tokens[p..].iter().all(|token| *token == ('*', false))
}

/// A glob that matches exactly `text`.
pub fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '*' | '?' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

pub fn matches(when: &RuleConditions, subject: &Subject) -> bool {
    let text = |pattern: &Option<String>, value: Option<&str>| match pattern {
        None => true,
        Some(pattern) => value.is_some_and(|value| glob(pattern, value)),
    };
    text(&when.host, subject.host.as_deref())
        && when.env.is_none_or(|env| subject.env == Some(env))
        && when
            .tag
            .as_ref()
            .is_none_or(|tag| subject.tags.iter().any(|t| t == tag))
        && text(&when.client, Some(&subject.client.name))
        && text(&when.method, Some(&subject.method))
        && text(&when.command, subject.command.as_deref().map(str::trim))
        && when.path.as_ref().is_none_or(|pattern| {
            subject
                .path
                .iter()
                .chain(&subject.paths)
                .any(|p| glob(pattern, p))
        })
        && when.min_risk.is_none_or(|min| subject.risk >= min)
        && when.max_risk.is_none_or(|max| subject.risk <= max)
}

/// Built-in policy: the desktop app and read-only calls are allowed; user-space changes
/// need approval on `prod`; system changes always need approval; destructive calls need
/// approval, and are refused on `prod`.
pub fn default_decision(subject: &Subject) -> Decision {
    if is_desktop(&subject.client) {
        return Decision::Allow;
    }
    let prod = subject.env == Some(HostEnv::Prod);
    match subject.risk {
        Risk::L0 => Decision::Allow,
        Risk::L1 if prod => Decision::Approve,
        Risk::L1 => Decision::Allow,
        Risk::L2 => Decision::Approve,
        Risk::L3 if prod => Decision::Deny,
        Risk::L3 => Decision::Approve,
    }
}

pub fn decide(rules: &[PolicyRule], granted: bool, subject: &Subject) -> Verdict {
    let matching: Vec<&PolicyRule> = rules.iter().filter(|r| matches(&r.when, subject)).collect();
    let find = |decision: Decision| matching.iter().find(|r| r.decision == decision);
    if let Some(rule) = find(Decision::Deny) {
        return Verdict {
            decision: Decision::Deny,
            matched: format!("rule {}", rule.id),
            approver: None,
        };
    }
    if let Some(rule) = find(Decision::Allow) {
        let approver = format!("rule {}", rule.id);
        return Verdict {
            decision: Decision::Allow,
            matched: approver.clone(),
            approver: Some(approver),
        };
    }
    if granted {
        return Verdict {
            decision: Decision::Allow,
            matched: "session grant".into(),
            approver: Some("session grant".into()),
        };
    }
    if let Some(rule) = find(Decision::Approve) {
        return Verdict {
            decision: Decision::Approve,
            matched: format!("rule {}", rule.id),
            approver: None,
        };
    }
    Verdict {
        decision: default_decision(subject),
        matched: "default".into(),
        approver: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject(risk: Risk, env: Option<HostEnv>) -> Subject {
        Subject {
            method: "exec.run".into(),
            host: Some("web-1".into()),
            env,
            tags: vec!["web".into()],
            client: ClientInfo::new("claude-code"),
            command: Some("systemctl restart nginx".into()),
            path: None,
            paths: vec!["/etc/nginx".into()],
            risk,
        }
    }

    fn rule(id: &str, decision: Decision, when: RuleConditions) -> PolicyRule {
        PolicyRule {
            id: id.into(),
            decision,
            when,
            note: None,
            created_at: 0,
            created_by: "cli".into(),
        }
    }

    #[test]
    fn globs() {
        assert!(glob("web-*", "web-1"));
        assert!(glob("*", ""));
        assert!(glob("/etc/*", "/etc/nginx/nginx.conf"));
        assert!(glob("systemctl restart ?ginx", "systemctl restart nginx"));
        assert!(!glob("web-?", "web-12"));
        assert!(glob(&escape("ls *.log"), "ls *.log"));
        assert!(!glob(&escape("ls *.log"), "ls a.log"));
        assert!(glob("a*b*c", "aXbYbZc"));
    }

    #[test]
    fn defaults_depend_on_risk_and_env() {
        let prod = Some(HostEnv::Prod);
        let decide_default = |risk, env| decide(&[], false, &subject(risk, env)).decision;
        assert_eq!(decide_default(Risk::L0, prod), Decision::Allow);
        assert_eq!(decide_default(Risk::L1, None), Decision::Allow);
        assert_eq!(decide_default(Risk::L1, prod), Decision::Approve);
        assert_eq!(
            decide_default(Risk::L2, Some(HostEnv::Dev)),
            Decision::Approve
        );
        assert_eq!(decide_default(Risk::L3, None), Decision::Approve);
        assert_eq!(decide_default(Risk::L3, prod), Decision::Deny);
        let mut desktop = subject(Risk::L3, prod);
        desktop.client = ClientInfo::new("desktop");
        assert_eq!(decide(&[], false, &desktop).decision, Decision::Allow);
    }

    #[test]
    fn deny_beats_allow_beats_grant_beats_approve() {
        let s = subject(Risk::L2, None);
        let allow = rule(
            "a",
            Decision::Allow,
            RuleConditions {
                command: Some("systemctl restart *".into()),
                ..Default::default()
            },
        );
        let deny = rule(
            "d",
            Decision::Deny,
            RuleConditions {
                tag: Some("web".into()),
                path: Some("/etc/*".into()),
                ..Default::default()
            },
        );
        let approve = rule(
            "p",
            Decision::Approve,
            RuleConditions {
                client: Some("claude-*".into()),
                ..Default::default()
            },
        );
        let verdict = decide(&[allow.clone(), approve.clone()], false, &s);
        assert_eq!(verdict.decision, Decision::Allow);
        assert_eq!(verdict.approver.as_deref(), Some("rule a"));
        assert_eq!(decide(&[allow, deny.clone()], false, &s).matched, "rule d");
        assert_eq!(
            decide(std::slice::from_ref(&approve), true, &s).matched,
            "session grant"
        );
        assert_eq!(decide(&[approve], false, &s).matched, "rule p");

        let low = rule(
            "low",
            Decision::Allow,
            RuleConditions {
                max_risk: Some(Risk::L1),
                ..Default::default()
            },
        );
        assert_eq!(decide(&[low], false, &s).matched, "default");
        let other_env = rule(
            "e",
            Decision::Deny,
            RuleConditions {
                env: Some(HostEnv::Prod),
                ..Default::default()
            },
        );
        assert_eq!(decide(&[other_env], false, &s).matched, "default");
    }
}
