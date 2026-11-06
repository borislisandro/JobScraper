//! Unix equivalent of `taskkill /T`: end a process together with everything it started.
//!
//! The scraper worker drives a browser through Playwright, and Playwright puts that browser in
//! its own process group, so signalling the worker's group would leave it running. Descendants
//! are found by parentage instead, read from /proc.
use std::collections::{HashMap, HashSet};

/// Parent pid out of a `/proc/<pid>/stat` line. The command name sits in parentheses and may
/// itself contain spaces or parentheses, so fields are counted from the last closing one.
fn parent_of(stat: &str) -> Option<u32> {
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(1)?.parse().ok()
}
fn children_by_parent() -> HashMap<u32, Vec<u32>> {
    let mut map: HashMap<u32, Vec<u32>> = HashMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return map;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let parent = std::fs::read_to_string(entry.path().join("stat"))
            .ok()
            .and_then(|stat| parent_of(&stat));
        if let Some(parent) = parent {
            map.entry(parent).or_default().push(pid);
        }
    }
    map
}
/// Every descendant of `root`, deepest first, so a parent never respawns a child we just ended.
fn descendants(root: u32, children: &HashMap<u32, Vec<u32>>) -> Vec<u32> {
    let mut order = Vec::new();
    let mut seen = HashSet::from([root]);
    let mut queue = vec![root];
    while let Some(pid) = queue.pop() {
        for child in children.get(&pid).into_iter().flatten() {
            if seen.insert(*child) {
                order.push(*child);
                queue.push(*child);
            }
        }
    }
    order.reverse();
    order
}
pub fn kill(root: u32) {
    // Stop the root first so it cannot start anything new while the tree is being read.
    signal(root, libc::SIGSTOP);
    // A process that forked between the scan and the kill is caught by the next pass.
    for _ in 0..3 {
        let found = descendants(root, &children_by_parent());
        if found.is_empty() {
            break;
        }
        for pid in found {
            signal(pid, libc::SIGKILL);
        }
    }
    signal(root, libc::SIGKILL);
}
fn signal(pid: u32, signal: libc::c_int) {
    // pid 0 and 1 would address a whole process group or init; never a worker.
    if pid > 1 {
        // SAFETY: kill(2) takes plain integers and has no memory-safety preconditions.
        unsafe {
            libc::kill(pid as libc::pid_t, signal);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parent_is_read_past_awkward_command_names() {
        assert_eq!(parent_of("1234 (node) S 77 1234 1234 0 -1"), Some(77));
        assert_eq!(parent_of("1234 (my (odd) name) S 88 1 1 0 -1"), Some(88));
        assert_eq!(parent_of("garbage"), None);
    }
    #[test]
    fn descendants_are_found_at_every_depth_deepest_first() {
        let children = HashMap::from([
            (10, vec![11, 12]),
            (11, vec![13]),
            (13, vec![14]),
            (99, vec![5]),
        ]);
        let found = descendants(10, &children);
        assert_eq!(found.len(), 4);
        assert!(!found.contains(&5) && !found.contains(&10));
        let depth = |pid| found.iter().position(|p| *p == pid).unwrap();
        assert!(depth(14) < depth(13) && depth(13) < depth(11));
    }
    #[test]
    fn killing_a_worker_takes_its_grandchildren_with_it() {
        use std::process::{Command, Stdio};
        // sh -> sh -> sleep: the middle shell stands in for a browser that is not in the
        // worker's process group.
        let mut worker = Command::new("sh")
            .args(["-c", "setsid sh -c 'sleep 60 & wait' & wait"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(400));
        let before = descendants(worker.id(), &children_by_parent());
        assert!(before.len() >= 2, "{before:?}");
        kill(worker.id());
        let _ = worker.wait();
        std::thread::sleep(std::time::Duration::from_millis(200));
        for pid in before {
            assert!(
                !std::path::Path::new(&format!("/proc/{pid}")).exists()
                    || std::fs::read_to_string(format!("/proc/{pid}/stat"))
                        .is_ok_and(|stat| stat.contains(") Z")),
                "{pid} survived"
            );
        }
    }
}
