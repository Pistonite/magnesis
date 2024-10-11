# magnesis
Pull (get it?) artifacts from GitHub action workflow for local testing

Originally part of the testing tool for [celer](https://github.com/Pistonite/celer),
extracted for general use.

## Install
```bash
cargo install magnesis --git https://github.com/Pistonite/magnesis
```

## Usage

You first need a GitHub personal access token (PAT) with repo read permissions. See [here](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/managing-your-personal-access-tokens#creating-a-fine-grained-personal-access-token)
This token is read from the environment variable `GITHUB_TOKEN`.

Download artifacts from a workflow ran on the current commit, in the current repository,
and extract them to `dist`:
```bash
magnesis
```
See `magnesis --help` for more options.

## Defaults
### Output
You can change the output directory with `-o`:
```bash
magnesis -o output
```
### Repository
By default, calls `git remote get-url origin` to get the repository URL, and parses it to get the owner and repository name
if it's in the form `http(s)://github.com/OWNER/REPO(.git)` or `git@github.com:OWNER/REPO(.git)`.
```bash
magnesis --repo foo/bar
```
If `origin` is not the remote to use or the URL is not in the expected format, you can specify the repository with the `--repo` flag
(for example, when there are multiple remotes).

### Commit
By default, calls `git rev-parse HEAD` to get the current commit. To use another commit, you can specify it with the `--rev` flag.
```bash
magnesis --rev foo
```
This will call `git rev-parse foo` to get the commit hash.

However, if `foo` already looks like a full commit hash, it will be used as-is.
This is useful when downloading artifacts from another repo.
