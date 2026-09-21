use std::collections::BTreeSet;

pub const SERVICE_DIRS: &[&str] = &["frontend", "backend", "web", "server", "api", "client"];
pub const SERVICE_PAIRS: [[&str; 2]; 4] = [
    ["frontend", "backend"],
    ["web", "server"],
    ["web", "api"],
    ["client", "server"],
];

/// Trusted, compile-time naming policy over an already bounded directory snapshot.
/// The scanner retains ownership of scope, collection guards, evidence and confirmation.
pub trait GroupingRule {
    fn is_service_directory(&self, name: &str) -> bool;
    fn sibling_pair(&self, directories: &BTreeSet<String>) -> Option<[String; 2]>;
    /// All possible pairs may need warnings even when no single merge can be suggested.
    fn sibling_pairs(&self, directories: &BTreeSet<String>) -> Vec<[String; 2]> {
        self.sibling_pair(directories).into_iter().collect()
    }
}

pub struct ConventionalGrouping;
pub static CONVENTIONAL_GROUPING: ConventionalGrouping = ConventionalGrouping;

impl GroupingRule for ConventionalGrouping {
    fn is_service_directory(&self, name: &str) -> bool {
        SERVICE_DIRS.iter().any(|n| n.eq_ignore_ascii_case(name))
    }

    fn sibling_pair(&self, directories: &BTreeSet<String>) -> Option<[String; 2]> {
        let mut pairs = self.sibling_pairs(directories).into_iter();
        let pair = pairs.next()?;
        // Competing boundaries need a user decision, not a rule priority ladder.
        pairs.next().is_none().then_some(pair)
    }

    fn sibling_pairs(&self, directories: &BTreeSet<String>) -> Vec<[String; 2]> {
        // Match ASCII conventions without rewriting actual paths. Case-colliding names
        // remain competing pairs on case-sensitive filesystems, never an arbitrary winner.
        SERVICE_PAIRS
            .iter()
            .flat_map(|pair| {
                directories
                    .iter()
                    .filter(move |n| n.eq_ignore_ascii_case(pair[0]))
                    .flat_map(move |a| {
                        directories
                            .iter()
                            .filter(move |n| n.eq_ignore_ascii_case(pair[1]))
                            .map(move |b| [a.clone(), b.clone()])
                    })
            })
            .take(17)
            .collect() // Core rejects more than 16; bound even malformed layouts.
    }
}
