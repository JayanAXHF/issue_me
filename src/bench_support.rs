use crate::ui::components::{
    issue_conversation::render_markdown_lines, issue_list::build_issue_body_preview,
};
use ratatui::text::Line;
use textwrap::Options;

pub fn render_markdown_for_bench(text: &str, width: usize, indent: usize) -> Vec<Line<'static>> {
    render_markdown_lines(text, width, indent)
}

pub fn build_issue_body_preview_for_bench(body_text: &str, width: usize) -> String {
    build_issue_body_preview(body_text, Options::new(width))
}

pub fn issue_body_fixture(repeat: usize) -> String {
    let paragraph = "This issue body mixes plain text, unicode like cafe and naive width tests, long URLs such as https://example.com/some/really/long/path/with/query?alpha=1&beta=2, and enough prose to trigger multi-line wrapping in the issue list preview. ";
    paragraph.repeat(repeat)
}

pub fn markdown_fixture(repeat: usize) -> String {
    let section = r#"# Hot Path Markdown

This fixture exercises **bold text**, _emphasis_, `inline code`, [links](https://github.com/jayanaxhf/gitv), and long prose that must wrap across multiple rendered lines inside the TUI.

> [!NOTE]
> Notes should render with a title and wrapped body content.

- first bullet with a very long explanation that wraps onto the next line
- second bullet with a reference to `IssueConversation`

```rust
fn render_preview(width: usize) -> Vec<String> {
    (0..width.min(4)).map(|n| format!("line-{n}")).collect()
}
```

| column | value |
| ------ | ----- |
| width  | 80    |
| indent | 2     |

Trailing paragraph to keep textwrap and markdown layout busy.

"#;
    section.repeat(repeat)
}
