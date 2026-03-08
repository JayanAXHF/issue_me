use octocrab::models::{Author, IssueState, Label, issues::Issue};
use slotmap::{SlotMap, new_key_type};
use std::collections::HashMap;

new_key_type! { pub struct AuthorId; }
new_key_type! { pub struct IssueId; }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StrId(u32);

#[derive(Debug, Clone)]
pub struct UiAuthor {
    pub github_id: u64,
    pub login: StrId,
}

#[derive(Debug, Clone)]
pub struct UiIssue {
    pub number: u64,
    pub state: IssueState,
    pub title: StrId,
    pub body: Option<StrId>,
    pub author: AuthorId,
    pub created_ts: i64,
    pub created_at_short: StrId,
    pub created_at_full: StrId,
    pub updated_at_short: StrId,
    pub comments: u32,
    pub assignees: Vec<AuthorId>,
    pub milestone: Option<StrId>,
    pub is_pull_request: bool,
    pub pull_request_url: Option<StrId>,
    pub labels: Vec<Label>,
}

impl UiIssue {
    pub fn from_octocrab(issue: &Issue, pool: &mut UiIssuePool) -> Self {
        let created_at_short = issue.created_at.format("%Y-%m-%d %H:%M").to_string();
        let created_at_full = issue.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
        let updated_at_short = issue.updated_at.format("%Y-%m-%d %H:%M").to_string();
        Self {
            number: issue.number,
            state: issue.state.clone(),
            title: pool.intern_str(issue.title.as_str()),
            body: issue.body.as_deref().map(|body| pool.intern_str(body)),
            author: pool.intern_author(&issue.user),
            created_ts: issue.created_at.timestamp(),
            created_at_short: pool.intern_str(created_at_short.as_str()),
            created_at_full: pool.intern_str(created_at_full.as_str()),
            updated_at_short: pool.intern_str(updated_at_short.as_str()),
            comments: issue.comments,
            assignees: issue
                .assignees
                .iter()
                .map(|assignee| pool.intern_author(assignee))
                .collect(),
            milestone: issue
                .milestone
                .as_ref()
                .map(|milestone| pool.intern_str(milestone.title.as_str())),
            is_pull_request: issue.pull_request.is_some(),
            pull_request_url: issue
                .pull_request
                .as_ref()
                .map(|pr| pool.intern_str(pr.html_url.as_str())),
            labels: issue.labels.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Span {
    start: u32,
    end: u32,
}

#[derive(Debug, Clone, Copy)]
struct Node {
    str: Option<u32>,
    first_link: u32,
}

#[derive(Debug, Clone, Copy)]
struct Link {
    byte: u8,
    node: u32,
}

#[derive(Debug, Default)]
struct Layer {
    links: Vec<Link>,
}

/// Courtesy of this amazing article by
/// [@matklad](https://matklad.github.io/2020/03/22/fast-simple-rust-interner.html)
#[derive(Debug)]
pub struct TrieStringInterner {
    trie: Vec<Node>,
    links: Vec<Layer>,
    strs: Vec<Span>,
    buf: String,
}

impl Default for TrieStringInterner {
    fn default() -> Self {
        Self {
            trie: vec![Node {
                str: None,
                first_link: 0,
            }],
            links: vec![Layer::default()],
            strs: Vec::new(),
            buf: String::new(),
        }
    }
}

impl TrieStringInterner {
    pub fn intern(&mut self, value: &str) -> StrId {
        let mut node_idx = 0_u32;
        for &byte in value.as_bytes() {
            let next = self.find_or_insert_child(node_idx, byte);
            node_idx = next;
        }

        let node = &mut self.trie[node_idx as usize];
        if let Some(existing) = node.str {
            return StrId(existing);
        }

        let start = u32::try_from(self.buf.len()).expect("interner buffer exceeded u32::MAX");
        self.buf.push_str(value);
        let end = u32::try_from(self.buf.len()).expect("interner buffer exceeded u32::MAX");

        let id = u32::try_from(self.strs.len()).expect("interner string table exceeded u32::MAX");
        self.strs.push(Span { start, end });
        node.str = Some(id);
        StrId(id)
    }

    pub fn resolve(&self, id: StrId) -> &str {
        let span = self
            .strs
            .get(id.0 as usize)
            .expect("attempted to resolve an unknown string id");
        &self.buf[span.start as usize..span.end as usize]
    }

    fn alloc_node(&mut self) -> u32 {
        let layer_idx =
            u32::try_from(self.links.len()).expect("interner layer table exceeded u32::MAX");
        self.links.push(Layer::default());
        let node_idx = u32::try_from(self.trie.len()).expect("interner trie exceeded u32::MAX");
        self.trie.push(Node {
            str: None,
            first_link: layer_idx,
        });
        node_idx
    }

    fn find_or_insert_child(&mut self, node_idx: u32, byte: u8) -> u32 {
        let layer_idx = self.trie[node_idx as usize].first_link;
        let layer = &mut self.links[layer_idx as usize];

        match layer.links.binary_search_by_key(&byte, |link| link.byte) {
            Ok(found) => layer.links[found].node,
            Err(insert_at) => {
                let child = self.alloc_node();
                let layer = &mut self.links[layer_idx as usize];
                layer.links.insert(insert_at, Link { byte, node: child });
                child
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TrieStringInterner;
    use octocrab::models::IssueState;

    use crate::ui::testing::{DummyDataConfig, dummy_ui_data_with};

    #[test]
    fn trie_interner_reuses_existing_string_ids() {
        let mut interner = TrieStringInterner::default();
        let one = interner.intern("issue-123");
        let two = interner.intern("issue-123");
        assert_eq!(one, two);
        assert_eq!(interner.resolve(one), "issue-123");
    }

    #[test]
    fn trie_interner_handles_prefixes() {
        let mut interner = TrieStringInterner::default();
        let short = interner.intern("open");
        let long = interner.intern("opened");
        assert_ne!(short, long);
        assert_eq!(interner.resolve(short), "open");
        assert_eq!(interner.resolve(long), "opened");
    }

    #[test]
    fn upsert_issue_reuses_existing_id() {
        let mut data = dummy_ui_data_with(DummyDataConfig {
            issue_count: 1,
            author_count: 2,
            comments_per_issue: 0,
            timeline_events_per_issue: 0,
            seed: 7,
        });
        let first_id = data.issue_ids[0];
        let mut second = data.pool.get_issue(first_id).clone();
        second.state = IssueState::Closed;
        second.title = data.pool.intern_str("closed issue");
        let second_id = data.pool.upsert_issue(second);

        assert_eq!(first_id, second_id);
        let stored = data.pool.get_issue(second_id);
        assert_eq!(stored.state, IssueState::Closed);
        assert_eq!(data.pool.resolve_str(stored.title), "closed issue");
    }
}

#[derive(Debug)]
pub struct UiIssuePool {
    strings: TrieStringInterner,
    authors: SlotMap<AuthorId, UiAuthor>,
    author_by_github_id: HashMap<u64, AuthorId>,
    issues: SlotMap<IssueId, UiIssue>,
    issue_by_number: HashMap<u64, IssueId>,
}

impl Default for UiIssuePool {
    fn default() -> Self {
        Self {
            strings: TrieStringInterner::default(),
            authors: SlotMap::with_key(),
            author_by_github_id: HashMap::new(),
            issues: SlotMap::with_key(),
            issue_by_number: HashMap::new(),
        }
    }
}

impl UiIssuePool {
    pub fn intern_str(&mut self, value: &str) -> StrId {
        self.strings.intern(value)
    }

    pub fn resolve_str(&self, id: StrId) -> &str {
        self.strings.resolve(id)
    }

    pub fn resolve_opt_str(&self, id: Option<StrId>) -> Option<&str> {
        id.map(|id| self.resolve_str(id))
    }

    pub fn intern_author(&mut self, author: &Author) -> AuthorId {
        let github_id = author.id.0;
        if let Some(existing) = self.author_by_github_id.get(&github_id).copied() {
            return existing;
        }

        let login = self.intern_str(author.login.as_str());
        let key = self.authors.insert(UiAuthor { github_id, login });
        self.author_by_github_id.insert(github_id, key);
        key
    }

    pub fn author_login(&self, author: AuthorId) -> &str {
        let author = self
            .authors
            .get(author)
            .expect("attempted to resolve an unknown author id");
        self.resolve_str(author.login)
    }

    #[cfg(test)]
    pub(crate) fn intern_test_author(&mut self, github_id: u64, login: &str) -> AuthorId {
        if let Some(existing) = self.author_by_github_id.get(&github_id).copied() {
            return existing;
        }

        let login = self.intern_str(login);
        let key = self.authors.insert(UiAuthor { github_id, login });
        self.author_by_github_id.insert(github_id, key);
        key
    }

    pub fn insert_issue(&mut self, issue: UiIssue) -> IssueId {
        self.upsert_issue(issue)
    }

    pub fn upsert_issue(&mut self, issue: UiIssue) -> IssueId {
        if let Some(existing) = self.issue_by_number.get(&issue.number).copied()
            && let Some(slot) = self.issues.get_mut(existing)
        {
            *slot = issue;
            return existing;
        }
        let number = issue.number;
        let issue_id = self.issues.insert(issue);
        self.issue_by_number.insert(number, issue_id);
        issue_id
    }

    pub fn get_issue(&self, issue_id: IssueId) -> &UiIssue {
        self.issues
            .get(issue_id)
            .expect("attempted to resolve an unknown issue id")
    }

    pub fn get_issue_mut(&mut self, issue_id: IssueId) -> &mut UiIssue {
        self.issues
            .get_mut(issue_id)
            .expect("attempted to resolve an unknown issue id")
    }
}
