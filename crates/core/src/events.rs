//! What happened to downloads, and how to say it.

use std::path::PathBuf;

/// A download that has just finished, successfully or not.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: u64,
    pub name: String,
    /// The finished file; `None` after a failure.
    pub path: Option<PathBuf>,
    pub dir: Option<PathBuf>,
    pub error: Option<String>,
}

impl Event {
    pub fn failed(&self) -> bool {
        self.error.is_some()
    }
}

#[derive(Debug, PartialEq)]
pub struct Message {
    pub title: String,
    pub body: String,
}

const NAMES_SHOWN: usize = 3;

fn short(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        text.to_string()
    } else {
        text.chars().take(limit - 1).chain(['…']).collect()
    }
}

fn describe(events: &[&Event], failed: bool) -> Option<Message> {
    let first = events.first()?;
    if events.len() == 1 {
        let name = short(&first.name, 80);
        return Some(match &first.error {
            Some(why) => Message { title: "Download failed".into(), body: format!("{name}\n{}", short(why, 140)) },
            None => Message { title: "Download complete".into(), body: name },
        });
    }
    let mut names: Vec<String> = events.iter().take(NAMES_SHOWN).map(|e| short(&e.name, 60)).collect();
    if events.len() > NAMES_SHOWN {
        names.push(format!("and {} more", events.len() - NAMES_SHOWN));
    }
    let what = if failed { "failed" } else { "complete" };
    Some(Message { title: format!("{} downloads {what}", events.len()), body: names.join("\n") })
}

/// One message for the successes and one for the failures, so that a bulk
/// download does not raise a notification per file.
pub fn summarize(events: &[Event], completed: bool, failures: bool) -> Vec<Message> {
    let pick = |failed: bool| events.iter().filter(|e| e.failed() == failed).collect::<Vec<_>>();
    let mut out = Vec::new();
    if completed {
        out.extend(describe(&pick(false), false));
    }
    if failures {
        out.extend(describe(&pick(true), true));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str, error: Option<&str>) -> Event {
        Event { id: 1, name: name.into(), path: None, dir: None, error: error.map(str::to_string) }
    }

    #[test]
    fn one_file_is_named() {
        let m = summarize(&[event("a.zip", None)], true, true);
        assert_eq!(m, [Message { title: "Download complete".into(), body: "a.zip".into() }]);
    }

    #[test]
    fn many_files_are_grouped_by_outcome() {
        let mut all: Vec<Event> = (1..=5).map(|i| event(&format!("{i}.mp3"), None)).collect();
        all.push(event("x.iso", Some("server replied 404")));
        let m = summarize(&all, true, true);
        assert_eq!(m[0].title, "5 downloads complete");
        assert_eq!(m[0].body, "1.mp3\n2.mp3\n3.mp3\nand 2 more");
        assert_eq!(m[1].body, "x.iso\nserver replied 404");
    }

    #[test]
    fn each_kind_can_be_switched_off() {
        let all = [event("a", None), event("b", Some("no"))];
        assert_eq!(summarize(&all, false, true).len(), 1);
        assert_eq!(summarize(&all, true, false)[0].title, "Download complete");
        assert!(summarize(&all, false, false).is_empty());
    }

    #[test]
    fn long_text_is_cut_on_a_character_boundary() {
        let m = summarize(&[event(&"é".repeat(200), None)], true, true);
        assert_eq!(m[0].body.chars().count(), 80);
    }
}
