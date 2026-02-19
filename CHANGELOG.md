# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/JayanAXHF/gitv/releases/tag/gitv-tui-v0.1.0) - 2026-02-19

### Added

- fix the text area paste in Issue Conversation and Issue Create
- add support for hyperlinks that don't break
- add support for quoting messages
- add a color picker for the label creation menu
- add an init script for the pre-push hooks
- add external editor support for editing comments
- add a fullscreen mode to issue conversation
- add proper error capturing at the top level
- show the title in the issue convo
- add a screenshot in the readme
- Add a build script with vergen and overhauled the titlebar
- add the ability to close issues
- Add syntect based syntax highlighting for fenced code blocks
- add regex based label search
- [**breaking**] Add an MVP for a issue creation screen
- add a mvp for assigning and removing assignees
- add log-level support
- *(reactions)* add support for adding and removing reactions
- *(reactions)* add support for showing reactions without slowing down FCP
- update status bar with help and quit hints
- Add a help menu
- *(QoL)* add universal quit
- *(md)* add support for admonitions
- add a body pane to make all comments visible
- add number based navigation
- added issue comment preview
- added issue count to the status line
- add markdown parsing for the comments
- add issue conversaion support with commenting
- add issue details
- add removing and adding labels
- add cursors and proper focus colors
- added throbber and made initial fetch non-blocking
- almost finished with the issue list basics
- scaffold initial components structure and playtest widget
- init

### Fixed

- fix links not having space around them
- fix issue conversation focus capture
- fix label creation not capturing focus properly
- fix typing in issue create clashing with other event handlers
- remove duplicate entry from Cargo.toml
- *(typo)* fix grep's full name
- fix admonition in readme
- fix syntax error
- fixed heading rendering
- fix another size mismatch issue
- fix reaction picker by making it "focus" on the selected reaction
- *(lint)* reduce the size of the `AppError` enum
- fix search bar appending issues instead of replacing them.
- fix logic bugs in number nav and dropdown rendering
- fix bug regarding conversation rendering
- made it not show PRs

### Other

- change name AGAIN to work aronud existing package
- add release-plz workflow
- add version specifier for hyperrat dep
- [**breaking**] Move the Link widget to its own crate
- update readme
- update README with init.py instructions
- add man pages via clap mangen
- rename bin from main to gitv
- update readme
- update .gitignore to ignore gifs
- add a gif for the demo
- fix clippy lint
- *(legal)* Add license(s)
- fix readme's format again
- remove bloat
- change service name from issue_me to gitv
- fix up the README's styles
- add a typos-ci action
- add a README and a KEYBINDS.md file
- make CLI help colorful
- add documentation to the CLI
- add _typos.toml
- [**breaking**] change the name to gitv
- change info to trace to not clutter logs
- [**breaking**] Overhauled the error handling to move away from `unwrap`
- remove dead code
- change IssuePreview to be a dumb component
- hoist CLI parsing into main function for better handling of special args
- [**breaking**] change help to look and be much better
- *(deps)* upgrade to pulldown_cmark v0.13.0
- *(issue list)* change issue list to search for issues rather than filtering manually
- refactor!(cli): add print_log_dir option to cli
- *(logging)* change logging to output to proper output dir instead of current dir
- change workflow to deny all lints
- create CI
- setup good release profile build settings
- *(clippy)* fix clippy lints
- *(gitignore)* ignore .log file
- fix clippy lints
- made issue list and preview more performant
- switched to dirty rendering and removed useless Render action
- removed bloat
