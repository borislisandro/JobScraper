//! Deterministic offline scoring. The production embedding provider writes 384 f32 values
//! from the bundled bge-small-en-v1.5 model; this pure layer is kept separately for testing.
use serde::Serialize;
use std::collections::HashSet;
#[derive(Clone)]
pub struct PersonaProfile {
    pub titles: Vec<String>,
    pub skills: Vec<String>,
    pub include: Vec<String>,
    pub include_mode: String,
    pub exclude: Vec<String>,
    pub location: Option<String>,
    pub work_mode: Option<String>,
    pub seniority: Option<String>,
    pub salary_min: Option<f64>,
    pub unknown_policy: String,
}
pub struct JobProfile {
    pub title: String,
    pub location: Option<String>,
    pub work_mode: Option<String>,
    pub seniority: Option<String>,
    pub salary_min: Option<f64>,
    pub description: String,
    pub skills: Vec<String>,
}
#[derive(Serialize)]
pub struct Score {
    pub total: f64,
    pub eligible: bool,
    pub filters: Vec<String>,
    pub components: Components,
    pub evidence: Evidence,
}
#[derive(Serialize)]
pub struct Components {
    pub semantic: f64,
    pub title: f64,
    pub required_skills: f64,
    pub preferred_keywords: f64,
}
#[derive(Serialize)]
pub struct Evidence {
    matched_skills: Vec<String>,
    missing_skills: Vec<String>,
    warnings: Vec<String>,
}
fn words(s: &str) -> HashSet<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '+')
        .filter(|s| s.len() > 1)
        .map(String::from)
        .collect()
}
fn ratio(a: &str, b: &str) -> f64 {
    let a = words(a);
    let b = words(b);
    if a.is_empty() || b.is_empty() {
        0.0
    } else {
        (a.intersection(&b).count() as f64) / (a.len().min(b.len()) as f64)
    }
}
fn whole_phrase(haystack: &str, needle: &str) -> bool {
    let h = format!(" {} ", haystack.to_lowercase());
    h.contains(&format!(" {} ", needle.to_lowercase()))
}
pub fn score(p: &PersonaProfile, j: &JobProfile) -> Score {
    score_with_similarity(p, j, None, None)
}
pub fn score_with_similarity(
    p: &PersonaProfile,
    j: &JobProfile,
    semantic: Option<f64>,
    title_similarity: Option<f64>,
) -> Score {
    let mut filters = Vec::new();
    let mut warnings = Vec::new();
    let hay = format!("{} {} {}", j.title, j.description, j.skills.join(" "));
    let excluded = p.exclude.iter().find(|x| whole_phrase(&hay, x));
    if let Some(x) = excluded {
        filters.push(format!("Excluded phrase: {x}"));
        return result(
            false,
            filters,
            warnings,
            0.,
            0.,
            0.,
            0.,
            vec![],
            p.skills.clone(),
        );
    }
    let include_matches = if p.include_mode == "all" {
        p.include.iter().all(|x| whole_phrase(&hay, x))
    } else {
        p.include.iter().any(|x| whole_phrase(&hay, x))
    };
    if !p.include.is_empty() && !include_matches {
        filters.push("No include keyword matched".into());
        return result(
            false,
            filters,
            warnings,
            0.,
            0.,
            0.,
            0.,
            vec![],
            p.skills.clone(),
        );
    }
    for (wanted, actual, label) in [
        (&p.location, &j.location, "location"),
        (&p.work_mode, &j.work_mode, "work mode"),
        (&p.seniority, &j.seniority, "seniority"),
    ] {
        if let Some(w) = wanted {
            match actual {
                Some(a) if !a.eq_ignore_ascii_case(w) => {
                    filters.push(format!("Known {label} conflict"));
                    return result(
                        false,
                        filters,
                        warnings,
                        0.,
                        0.,
                        0.,
                        0.,
                        vec![],
                        p.skills.clone(),
                    );
                }
                None if p.unknown_policy == "require_known" => {
                    filters.push(format!("Unknown required {label}"));
                    return result(
                        false,
                        filters,
                        warnings,
                        0.,
                        0.,
                        0.,
                        0.,
                        vec![],
                        p.skills.clone(),
                    );
                }
                None => warnings.push(format!("Unknown {label} passed")),
                _ => {}
            }
        }
    }
    if let Some(wanted) = p.salary_min {
        match j.salary_min {
            Some(actual) if actual < wanted => {
                filters.push("Known salary conflict".into());
                return result(
                    false,
                    filters,
                    warnings,
                    0.,
                    0.,
                    0.,
                    0.,
                    vec![],
                    p.skills.clone(),
                );
            }
            None if p.unknown_policy == "require_known" => {
                filters.push("Unknown required salary".into());
                return result(
                    false,
                    filters,
                    warnings,
                    0.,
                    0.,
                    0.,
                    0.,
                    vec![],
                    p.skills.clone(),
                );
            }
            None => warnings.push("Unknown salary passed".into()),
            _ => {}
        }
    }
    let text_ratio = semantic.unwrap_or_else(|| {
        ratio(
            &format!("{} {}", p.titles.join(" "), p.skills.join(" ")),
            &j.description,
        )
    });
    let title = title_similarity.unwrap_or_else(|| {
        p.titles
            .iter()
            .map(|t| ratio(t, &j.title))
            .fold(0., f64::max)
    });
    let jobskills: HashSet<_> = j.skills.iter().map(|s| s.to_lowercase()).collect();
    let matched: Vec<String> = p
        .skills
        .iter()
        .filter(|s| jobskills.contains(&s.to_lowercase()) || whole_phrase(&hay, s))
        .cloned()
        .collect();
    let missing: Vec<String> = p
        .skills
        .iter()
        .filter(|s| !matched.contains(s))
        .cloned()
        .collect();
    let skill = if p.skills.is_empty() {
        0.0
    } else {
        matched.len() as f64 / p.skills.len() as f64
    };
    let preferred = if p.include.is_empty() {
        0.0
    } else {
        p.include.iter().filter(|s| whole_phrase(&hay, s)).count() as f64 / p.include.len() as f64
    };
    result(
        true, filters, warnings, text_ratio, title, skill, preferred, matched, missing,
    )
}
fn result(
    eligible: bool,
    filters: Vec<String>,
    warnings: Vec<String>,
    semantic: f64,
    title: f64,
    skills: f64,
    preferred: f64,
    matched: Vec<String>,
    missing: Vec<String>,
) -> Score {
    Score {
        total: ((semantic * 0.45 + title * 0.25 + skills * 0.2 + preferred * 0.1) * 1000.).round()
            / 10.,
        eligible,
        filters,
        components: Components {
            semantic: semantic * 100.,
            title: title * 100.,
            required_skills: skills * 100.,
            preferred_keywords: preferred * 100.,
        },
        evidence: Evidence {
            matched_skills: matched,
            missing_skills: missing,
            warnings,
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_passes_but_conflict_does_not() {
        let p = PersonaProfile {
            titles: vec!["Engineer".into()],
            skills: vec![],
            include: vec![],
            include_mode: "any".into(),
            exclude: vec![],
            location: Some("Lisbon".into()),
            work_mode: None,
            seniority: None,
            salary_min: None,
            unknown_policy: "pass".into(),
        };
        let unknown = JobProfile {
            title: "Engineer".into(),
            location: None,
            work_mode: None,
            seniority: None,
            salary_min: None,
            description: "engineering".into(),
            skills: vec![],
        };
        assert!(score(&p, &unknown).eligible);
        let conflict = JobProfile {
            location: Some("Remote".into()),
            ..unknown
        };
        assert!(!score(&p, &conflict).eligible)
    }
    #[test]
    fn include_mode_any_and_all_are_distinct() {
        let mut p = PersonaProfile {
            titles: vec![],
            skills: vec![],
            include: vec!["rust".into(), "embedded".into()],
            include_mode: "any".into(),
            exclude: vec![],
            location: None,
            work_mode: None,
            seniority: None,
            salary_min: None,
            unknown_policy: "pass".into(),
        };
        let j = JobProfile {
            title: "Rust engineer".into(),
            location: None,
            work_mode: None,
            seniority: None,
            salary_min: None,
            description: "Rust only".into(),
            skills: vec![],
        };
        assert!(score(&p, &j).eligible);
        p.include_mode = "all".into();
        assert!(!score(&p, &j).eligible);
    }
    #[test]
    fn hard_filters_cover_excludes_conflicts_and_unknown_policy() {
        let base = PersonaProfile {
            titles: vec![],
            skills: vec![],
            include: vec![],
            include_mode: "any".into(),
            exclude: vec!["security clearance".into()],
            location: Some("Lisbon".into()),
            work_mode: Some("remote".into()),
            seniority: Some("senior".into()),
            salary_min: Some(100.0),
            unknown_policy: "pass".into(),
        };
        let excluded = JobProfile {
            title: "Engineer".into(),
            location: None,
            work_mode: None,
            seniority: None,
            salary_min: None,
            description: "Requires security clearance".into(),
            skills: vec![],
        };
        assert!(!score(&base, &excluded).eligible);
        let conflict = JobProfile {
            title: "Engineer".into(),
            location: Some("Porto".into()),
            work_mode: Some("onsite".into()),
            seniority: Some("junior".into()),
            salary_min: Some(50.0),
            description: "normal".into(),
            skills: vec![],
        };
        assert!(
            !score(
                &PersonaProfile {
                    exclude: vec![],
                    ..base.clone()
                },
                &conflict
            )
            .eligible
        );
        let unknown = JobProfile {
            title: "Engineer".into(),
            location: None,
            work_mode: None,
            seniority: None,
            salary_min: None,
            description: "normal".into(),
            skills: vec![],
        };
        assert!(
            score(
                &PersonaProfile {
                    exclude: vec![],
                    ..base.clone()
                },
                &unknown
            )
            .eligible
        );
        assert!(
            !score(
                &PersonaProfile {
                    exclude: vec![],
                    unknown_policy: "require_known".into(),
                    ..base
                },
                &unknown
            )
            .eligible
        );
    }
    #[test]
    fn salary_unknown_passes_but_known_lower_conflicts() {
        let p = PersonaProfile {
            titles: vec![],
            skills: vec![],
            include: vec![],
            include_mode: "any".into(),
            exclude: vec![],
            location: None,
            work_mode: None,
            seniority: None,
            salary_min: Some(100.0),
            unknown_policy: "pass".into(),
        };
        let base_job = JobProfile {
            title: "Engineer".into(),
            location: None,
            work_mode: None,
            seniority: None,
            salary_min: None,
            description: "normal".into(),
            skills: vec![],
        };
        // Unknown salary passes (with a warning) under the default "pass" unknown policy.
        assert!(score(&p, &base_job).eligible);
        // A known-but-lower salary is a hard reject, not a warning.
        let lower = JobProfile {
            salary_min: Some(50.0),
            ..base_job
        };
        assert!(!score(&p, &lower).eligible);
    }
}
