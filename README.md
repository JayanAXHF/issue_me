# gitv

> [!NOTE]
>
> Following in the footsteps of the <u>`g`</u>lobal <u>`r`</u>egex <u>`e`</u>xpression <u>`p`</u>rint `grep`, I introduce to you <u>`g`</u>ithub <u>`i`</u>ssues <u>`t`</u>ui <u>`v`</u>iewer `gitv`.

`gitv` is a terminal-based viewer for GitHub issues. It allows you to view and manage your GitHub issues directly from the terminal.

![Made with VHS](https://vhs.charm.sh/vhs-WEyf76YrrGK3OgECWA3pI.gif)

### Features

- View issues from any GitHub repository
- View issue conversations, including parsed markdown content
- Full support for adding and removing reactions
- Regex search for labels, plus the ability to create, edit, add, and remove labels from issues
- Commenting on issues, with support for markdown formatting
- Closing issues
- Assigning and unassigning issues to users
- Creating new issues
- Syntax highlighting for code blocks in issue conversations

### Installation

#### Using git

1. Clone the repository:

```bash
git clone https://github.com/jayanaxhf/gitv.git
```

2. Navigate to the project directory:

```bash
cd gitv
```

3. Build the project:

```bash
cargo install --path .
```

#### Using cargo

Coming Soon!

### Usage

```
Usage: main [OPTIONS] [OWNER] [REPO]

Arguments:
  [OWNER]
          GitHub repository owner or organization (for example: `rust-lang`).

          This is required unless `--print-log-dir` or `--set-token` is provided.

  [REPO]
          GitHub repository name under `owner` (for example: `rust`).

          This is required unless `--print-log-dir` or `--set-token` is provided.

Options:
  -l, --log-level <LOG_LEVEL>
          Global logging verbosity used by the application logger.

          Defaults to `info`.

          [default: info]
          [possible values: trace, debug, info, warn, error, none]

  -p, --print-log-dir
          Prints the directory where log files are written and exits

  -s, --set-token <SET_TOKEN>
          Stores/updates the GitHub token in the configured credential store.

          When provided, this command updates the saved token value.

  -h, --help
          Print help (see a summary with '-h')

  -V, --version
          Print version
```

See [KEYBINDINGS.md](./KEYBINDS.md) for a list of keybindings used in the application.

### Token Security

`gitv` uses the `keyring` crate to securely store your GitHub token in your system's credential store. This means that your token is encrypted and protected by your operating system's security features, providing a secure way to manage your authentication credentials.

### Contributing

Contributions to `gitv` are welcome! If you have an idea for a new feature or have found a bug, please open an issue or submit a pull request on the GitHub repository.

### License

`gitv` is dual-licensed under the MIT License and the Unlicense, at your option. See the [MIT](./LICENSE-MIT) and [Unlicense](./UNLICENSE) for more information.
