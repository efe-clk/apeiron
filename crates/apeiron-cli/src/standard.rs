//! The facts of a compliance report, mapped to the controls of a standard
//! (docs/REPORTS.md §3.1). The mapping points to evidence; it is not a
//! certification.

use serde::Serialize;

use crate::FullReport;

/// The standards a report can be mapped to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Standard {
    /// ISO/IEC 27001:2022, Annex A
    Iso27001,
    /// SOC 2, the 2017 Trust Services Criteria
    Soc2,
}

/// What the report shows for a control.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    /// The report holds facts that support the control.
    Evidence,
    /// The report shows something the control asks for is missing.
    Gap,
    /// The report holds no facts on it; the option named in `facts` adds
    /// them.
    NotCovered,
}

#[derive(Serialize)]
pub(crate) struct Control {
    pub(crate) id: &'static str,
    pub(crate) title: &'static str,
    pub(crate) status: Status,
    pub(crate) facts: String,
}

#[derive(Serialize)]
pub(crate) struct Mapping {
    pub(crate) standard: &'static str,
    pub(crate) controls: Vec<Control>,
}

/// Maps `full` to the controls of `standard`.
pub(crate) fn map(standard: Standard, full: &FullReport) -> Mapping {
    match standard {
        Standard::Iso27001 => iso27001(full),
        Standard::Soc2 => soc2(full),
    }
}

fn control(id: &'static str, title: &'static str, status: Status, facts: String) -> Control {
    Control {
        id,
        title,
        status,
        facts,
    }
}

fn iso27001(full: &FullReport) -> Mapping {
    use Status::{Evidence, Gap, NotCovered};
    let r = &full.report;
    let web = full.web.as_ref();
    let no_web = || "not in this report; add `--web`".to_owned();
    let mut controls = Vec::new();

    controls.push(match web {
        None => control("A.5.15", "Access control", NotCovered, no_web()),
        Some(w) => {
            let count = |role| w.users.iter().filter(|u| u.role == role).count();
            control(
                "A.5.15",
                "Access control",
                Evidence,
                format!(
                    "{} users: {} admins, {} members, {} readers; members and readers reach only the backups listed for them",
                    w.users.len(),
                    count(apeiron_server::Role::Admin),
                    count(apeiron_server::Role::Member),
                    count(apeiron_server::Role::Reader)
                ),
            )
        }
    });

    controls.push(match web {
        None => control("A.5.17", "Authentication information", NotCovered, no_web()),
        Some(w) => {
            let weak: Vec<&str> = w
                .users
                .iter()
                .filter(|u| {
                    u.role == apeiron_server::Role::Admin && u.oidc.is_none() && !u.second_factor
                })
                .map(|u| u.name.as_str())
                .collect();
            let facts = format!(
                "passwords are kept as Argon2id hashes; {} users sign in through the identity provider, {} with an authenticator code",
                w.users.iter().filter(|u| u.oidc.is_some()).count(),
                w.users.iter().filter(|u| u.second_factor).count()
            );
            if weak.is_empty() {
                control("A.5.17", "Authentication information", Evidence, facts)
            } else {
                control(
                    "A.5.17",
                    "Authentication information",
                    Gap,
                    format!(
                        "{facts}; admins with a password alone: {}",
                        weak.join(", ")
                    ),
                )
            }
        }
    });

    let kept = r.refs.iter().filter(|f| f.retention_days.is_some()).count();
    let held = r.holds.iter().filter(|h| h.lift.is_none()).count();
    controls.push(if r.retention.is_empty() && held == 0 {
        control(
            "A.5.33",
            "Protection of records",
            Gap,
            "no retention rule and no legal hold; any backup can be deleted".to_owned(),
        )
    } else {
        control(
            "A.5.33",
            "Protection of records",
            Evidence,
            format!(
                "{} retention rules keep {kept} of {} refs; {held} legal holds are in place",
                r.retention.len(),
                r.refs.len()
            ),
        )
    });

    let stale: Vec<&str> = r
        .refs
        .iter()
        .filter(|f| f.stale)
        .map(|f| f.name.as_str())
        .collect();
    let verified = match &r.verify {
        Some(v) if v.is_ok() => {
            format!("all {} stored objects read and checked", v.objects_checked)
        }
        Some(_) => "the check found damaged data".to_owned(),
        None => "stored data not checked in this report; add `--verify`".to_owned(),
    };
    let damaged = r.verify.as_ref().is_some_and(|v| !v.is_ok());
    controls.push(if r.refs.is_empty() || !stale.is_empty() || damaged {
        let why = if r.refs.is_empty() {
            "no backups".to_owned()
        } else if stale.is_empty() {
            verified
        } else {
            format!("stale backups: {}; {verified}", stale.join(", "))
        };
        control("A.8.13", "Information backup", Gap, why)
    } else {
        control(
            "A.8.13",
            "Information backup",
            Evidence,
            format!("{} backups, none stale; {verified}", r.refs.len()),
        )
    });

    controls.push(match web {
        None => control("A.8.15", "Logging", NotCovered, no_web()),
        Some(w) if w.audit.whole => control(
            "A.8.15",
            "Logging",
            Evidence,
            format!(
                "audit log of {} events with a whole chain of hashes; last hash {}",
                w.audit.events,
                w.audit.last.as_deref().unwrap_or("—")
            ),
        ),
        Some(w) => control(
            "A.8.15",
            "Logging",
            Gap,
            format!(
                "the audit log's chain is broken: {}",
                w.audit.error.as_deref().unwrap_or("")
            ),
        ),
    });

    controls.push(if !r.repository.encrypted {
        control(
            "A.8.24",
            "Use of cryptography",
            Gap,
            "the repository is not encrypted".to_owned(),
        )
    } else {
        control(
            "A.8.24",
            "Use of cryptography",
            Evidence,
            format!(
                "the repository is encrypted, with {} passwords and {} recovery keys",
                r.repository.passwords, r.repository.recovery_keys
            ),
        )
    });

    let compared =
        r.refs.iter().any(|f| !f.copies.is_empty()) || r.remotes.iter().any(|m| m.error.is_some());
    let mut behind: Vec<String> = r
        .refs
        .iter()
        .flat_map(|f| {
            f.copies
                .iter()
                .filter(|c| c.state != apeiron::CopyKind::Current)
                .map(move |c| format!("{} on {}", f.name, c.remote))
        })
        .collect();
    behind.extend(
        r.remotes
            .iter()
            .filter(|m| m.error.is_some())
            .map(|m| format!("{} did not answer", m.name)),
    );
    controls.push(if r.remotes.is_empty() {
        control(
            "A.5.30",
            "ICT readiness for business continuity",
            Gap,
            "no remote copy; every backup exists in one place".to_owned(),
        )
    } else if !compared {
        control(
            "A.5.30",
            "ICT readiness for business continuity",
            NotCovered,
            "copies not compared in this report; add `--remotes`".to_owned(),
        )
    } else if behind.is_empty() {
        control(
            "A.5.30",
            "ICT readiness for business continuity",
            Evidence,
            format!("every backup is current on {} remotes", r.remotes.len()),
        )
    } else {
        control(
            "A.5.30",
            "ICT readiness for business continuity",
            Gap,
            format!("not current: {}", behind.join(", ")),
        )
    });

    Mapping {
        standard: "ISO/IEC 27001:2022 Annex A",
        controls,
    }
}

/// The criteria of SOC 2 that the facts behind an ISO/IEC 27001 control bear
/// on: the ISO control, then the criterion and a short title of our own.
const SOC2: [(&str, &str, &str); 7] = [
    ("A.5.17", "CC6.1", "Logical access security"),
    ("A.5.15", "CC6.3", "Access based on roles"),
    ("A.8.24", "CC6.7", "Protecting data that moves"),
    ("A.8.15", "CC7.2", "Monitoring"),
    ("A.5.30", "CC9.1", "Mitigating business disruption"),
    ("A.8.13", "A1.2", "Data backup and recovery"),
    ("A.5.33", "C1.1", "Keeping confidential information"),
];

/// The same facts as `iso27001`, so that the two reports cannot disagree.
fn soc2(full: &FullReport) -> Mapping {
    let iso = iso27001(full).controls;
    let controls = SOC2
        .iter()
        .filter_map(|(from, id, title)| {
            let c = iso.iter().find(|c| c.id == *from)?;
            Some(control(id, title, c.status, c.facts.clone()))
        })
        .collect();
    Mapping {
        standard: "SOC 2 Trust Services Criteria",
        controls,
    }
}

/// The Markdown section of a mapping.
pub(crate) fn print(mapping: &Mapping) {
    say!("");
    say!("## {}", mapping.standard);
    say!("");
    say!(
        "The facts of this report that bear on each control. This points to evidence; it is not a certification, and a control may need more than a backup system shows."
    );
    say!("");
    say!("| Control | Status | Facts |");
    say!("| --- | --- | --- |");
    for c in &mapping.controls {
        let status = match c.status {
            Status::Evidence => "evidence",
            Status::Gap => "gap",
            Status::NotCovered => "not covered",
        };
        say!("| {} {} | {status} | {} |", c.id, c.title, c.facts);
    }
}
