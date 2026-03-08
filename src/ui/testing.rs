use std::collections::HashMap;

use fake::{
    Fake,
    faker::{
        internet::en::Username,
        lorem::en::{Paragraph, Sentence, Words},
    },
    rand::{SeedableRng, prelude::IndexedRandom, rngs::StdRng},
};
use octocrab::models::{Event as IssueEvent, IssueState};

use crate::{
    bench_support::{issue_body_fixture, markdown_fixture},
    ui::{
        components::{
            issue_conversation::{CommentView, IssueConversationSeed, TimelineEventView},
            issue_detail::IssuePreviewSeed,
        },
        issue_data::{AuthorId, IssueId, UiIssue, UiIssuePool},
    },
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct DummyDataConfig {
    pub issue_count: usize,
    pub author_count: usize,
    pub comments_per_issue: usize,
    pub timeline_events_per_issue: usize,
    pub seed: u64,
}

impl Default for DummyDataConfig {
    fn default() -> Self {
        Self {
            issue_count: 32,
            author_count: 8,
            comments_per_issue: 4,
            timeline_events_per_issue: 3,
            seed: 42,
        }
    }
}

#[derive(Debug)]
pub(crate) struct DummyUiData {
    pub pool: UiIssuePool,
    pub issue_ids: Vec<IssueId>,
    pub issue_numbers: Vec<u64>,
    pub preview_seeds: HashMap<IssueId, IssuePreviewSeed>,
    pub conversation_seeds: HashMap<IssueId, IssueConversationSeed>,
    pub comments_by_issue_number: HashMap<u64, Vec<CommentView>>,
    pub timeline_by_issue_number: HashMap<u64, Vec<TimelineEventView>>,
}

impl DummyUiData {
    pub fn issue(&self, issue_id: IssueId) -> &UiIssue {
        self.pool.get_issue(issue_id)
    }

    pub fn preview_seed(&self, issue_id: IssueId) -> IssuePreviewSeed {
        self.preview_seeds
            .get(&issue_id)
            .cloned()
            .expect("dummy preview seed missing for issue")
    }

    pub fn conversation_seed(&self, issue_id: IssueId) -> IssueConversationSeed {
        self.conversation_seeds
            .get(&issue_id)
            .cloned()
            .expect("dummy conversation seed missing for issue")
    }

    pub fn comments_for_number(&self, number: u64) -> &[CommentView] {
        self.comments_by_issue_number
            .get(&number)
            .map(Vec::as_slice)
            .expect("dummy comments missing for issue number")
    }

    pub fn timeline_for_number(&self, number: u64) -> &[TimelineEventView] {
        self.timeline_by_issue_number
            .get(&number)
            .map(Vec::as_slice)
            .expect("dummy timeline missing for issue number")
    }
}

pub(crate) fn dummy_ui_data() -> DummyUiData {
    dummy_ui_data_with(DummyDataConfig::default())
}

pub(crate) fn dummy_ui_data_with(config: DummyDataConfig) -> DummyUiData {
    assert!(
        config.issue_count > 0,
        "issue_count must be greater than zero"
    );
    assert!(
        config.author_count > 0,
        "author_count must be greater than zero"
    );

    let mut rng = StdRng::seed_from_u64(config.seed);
    let mut pool = UiIssuePool::default();

    let authors: Vec<AuthorFixture> = (0..config.author_count)
        .map(|idx| {
            let login: String = Username().fake_with_rng(&mut rng);
            let github_id = 10_000 + idx as u64;
            let author_id = pool.intern_test_author(github_id, &login);
            AuthorFixture {
                github_id,
                login,
                author_id,
            }
        })
        .collect();

    let milestones: Vec<String> = (0..(config.author_count / 2).max(2))
        .map(|_| Sentence(2..4).fake_with_rng(&mut rng))
        .collect();

    let mut issue_ids = Vec::with_capacity(config.issue_count);
    let mut issue_numbers = Vec::with_capacity(config.issue_count);
    let mut preview_seeds = HashMap::with_capacity(config.issue_count);
    let mut conversation_seeds = HashMap::with_capacity(config.issue_count);
    let mut comments_by_issue_number = HashMap::with_capacity(config.issue_count);
    let mut timeline_by_issue_number = HashMap::with_capacity(config.issue_count);

    for idx in 0..config.issue_count {
        let issue_number = 1000 + idx as u64;
        let issue = make_issue(
            &mut pool,
            &authors,
            &milestones,
            issue_number,
            idx,
            &mut rng,
        );
        let issue_id = pool.insert_issue(issue);
        let stored = pool.get_issue(issue_id);

        issue_ids.push(issue_id);
        issue_numbers.push(issue_number);
        preview_seeds.insert(issue_id, IssuePreviewSeed::from_ui_issue(stored, &pool));
        conversation_seeds.insert(
            issue_id,
            IssueConversationSeed::from_ui_issue(stored, &pool),
        );
        comments_by_issue_number.insert(
            issue_number,
            make_comments(
                &authors,
                issue_number,
                config.comments_per_issue,
                idx,
                &mut rng,
            ),
        );
        timeline_by_issue_number.insert(
            issue_number,
            make_timeline_events(
                &authors,
                issue_number,
                config.timeline_events_per_issue,
                idx,
                &mut rng,
            ),
        );
    }

    DummyUiData {
        pool,
        issue_ids,
        issue_numbers,
        preview_seeds,
        conversation_seeds,
        comments_by_issue_number,
        timeline_by_issue_number,
    }
}

#[derive(Debug, Clone)]
struct AuthorFixture {
    github_id: u64,
    login: String,
    author_id: AuthorId,
}

fn make_issue(
    pool: &mut UiIssuePool,
    authors: &[AuthorFixture],
    milestones: &[String],
    issue_number: u64,
    idx: usize,
    rng: &mut StdRng,
) -> UiIssue {
    let author = authors
        .choose(rng)
        .map(|author| author.author_id)
        .expect("author fixture list should not be empty");
    let state = if idx % 5 == 0 {
        IssueState::Closed
    } else {
        IssueState::Open
    };
    let title = format!(
        "{} #{issue_number}",
        Sentence(3..6).fake_with_rng::<String, _>(rng)
    );
    let shared_fragment = if idx % 2 == 0 {
        issue_body_fixture(1)
    } else {
        markdown_fixture(1)
    };
    let tags: Vec<String> = Words(2..5).fake_with_rng(rng);
    let body = format!(
        "{}\n\n{}\n\nTags: {}",
        Paragraph(2..4).fake_with_rng::<String, _>(rng),
        shared_fragment,
        tags.join(", ")
    );
    let created_ts = 1_704_067_200_i64 + (idx as i64 * 3_600);
    let created_at_short = format_timestamp(created_ts, false);
    let created_at_full = format_timestamp(created_ts, true);
    let updated_at_short = format_timestamp(created_ts + 1_800, false);
    let milestone = (idx % 3 == 0).then(|| {
        let milestone = milestones
            .choose(rng)
            .expect("milestone fixture list should not be empty");
        pool.intern_str(milestone)
    });
    let assignee_count = 1 + (idx % authors.len().min(3).max(1));
    let assignees = authors
        .iter()
        .cycle()
        .skip(idx % authors.len())
        .take(assignee_count)
        .map(|author| author.author_id)
        .collect();
    let is_pull_request = idx % 4 == 0;
    let pull_request_url = if is_pull_request {
        let url = format!("https://github.com/example/repo/pull/{issue_number}");
        Some(pool.intern_str(&url))
    } else {
        None
    };

    UiIssue {
        number: issue_number,
        state,
        title: pool.intern_str(&title),
        body: Some(pool.intern_str(&body)),
        author,
        created_ts,
        created_at_short: pool.intern_str(&created_at_short),
        created_at_full: pool.intern_str(&created_at_full),
        updated_at_short: pool.intern_str(&updated_at_short),
        comments: 2 + (idx % 8) as u32,
        assignees,
        milestone,
        is_pull_request,
        pull_request_url,
        labels: Vec::new(),
    }
}

fn make_comments(
    authors: &[AuthorFixture],
    issue_number: u64,
    comment_count: usize,
    issue_idx: usize,
    rng: &mut StdRng,
) -> Vec<CommentView> {
    (0..comment_count)
        .map(|comment_idx| {
            let author = &authors[(issue_idx + comment_idx) % authors.len()];
            let created_ts = 1_704_067_200_i64 + (issue_idx as i64 * 7_200) + comment_idx as i64;
            CommentView {
                id: issue_number * 100 + comment_idx as u64,
                author: author.login.clone().into(),
                created_at: format_timestamp(created_ts, false).into(),
                created_ts,
                body: format!(
                    "{}\n\n{}",
                    Paragraph(1..3).fake_with_rng::<String, _>(rng),
                    issue_body_fixture(1)
                )
                .into(),
                reactions: None,
                my_reactions: None,
            }
        })
        .collect()
}

fn make_timeline_events(
    authors: &[AuthorFixture],
    issue_number: u64,
    event_count: usize,
    issue_idx: usize,
    rng: &mut StdRng,
) -> Vec<TimelineEventView> {
    const EVENTS: &[(IssueEvent, &str, &str)] = &[
        (IssueEvent::Assigned, "@", "assigned this issue"),
        (IssueEvent::Labeled, "#", "updated labels"),
        (IssueEvent::Closed, "x", "closed this issue"),
        (IssueEvent::Reopened, "+", "reopened this issue"),
        (IssueEvent::Referenced, "~", "referenced this issue"),
    ];

    (0..event_count)
        .map(|event_idx| {
            let author = &authors[(issue_idx + event_idx) % authors.len()];
            let (event, icon, action) = &EVENTS[(issue_idx + event_idx) % EVENTS.len()];
            let created_ts = 1_704_067_200_i64 + (issue_idx as i64 * 10_800) + event_idx as i64;
            let details = format!(
                "{}\n\n{}\n\nActor id: {}",
                Sentence(4..8).fake_with_rng::<String, _>(rng),
                markdown_fixture(1),
                author.github_id
            );

            TimelineEventView {
                id: issue_number * 1_000 + event_idx as u64,
                created_at: format_timestamp(created_ts, false).into(),
                created_ts,
                actor: author.login.clone().into(),
                event: event.clone(),
                icon,
                summary: format!("{} {}", author.login, action).into(),
                details: details.into(),
            }
        })
        .collect()
}

fn format_timestamp(ts: i64, include_seconds: bool) -> String {
    let day = 1 + (ts.div_euclid(86_400).rem_euclid(28) as u32);
    let hour = ts.div_euclid(3_600).rem_euclid(24) as u32;
    let minute = ts.div_euclid(60).rem_euclid(60) as u32;
    let second = ts.rem_euclid(60) as u32;

    if include_seconds {
        format!("2024-01-{day:02} {hour:02}:{minute:02}:{second:02}")
    } else {
        format!("2024-01-{day:02} {hour:02}:{minute:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::{DummyDataConfig, dummy_ui_data, dummy_ui_data_with};

    #[test]
    fn dummy_data_builds_expected_shapes() {
        let data = dummy_ui_data();
        let issue_id = data.issue_ids[0];
        let issue = data.issue(issue_id);

        assert_eq!(data.issue_ids.len(), 32);
        assert_eq!(data.issue_numbers.len(), 32);
        assert!(data.preview_seed(issue_id).number >= 1000);
        assert_eq!(data.conversation_seed(issue_id).number, issue.number);
        assert_eq!(data.comments_for_number(issue.number).len(), 4);
        assert_eq!(data.timeline_for_number(issue.number).len(), 3);
    }

    #[test]
    fn dummy_data_reuses_interned_author_strings() {
        let data = dummy_ui_data_with(DummyDataConfig {
            issue_count: 6,
            author_count: 2,
            comments_per_issue: 1,
            timeline_events_per_issue: 1,
            seed: 11,
        });

        let first = data.issue(data.issue_ids[0]);
        let second = data.issue(data.issue_ids[1]);
        let first_author = data.pool.author_login(first.author);
        let second_author = data.pool.author_login(second.author);

        assert!(!first_author.is_empty());
        if first.author == second.author {
            assert_eq!(first_author, second_author);
        }
    }
}
